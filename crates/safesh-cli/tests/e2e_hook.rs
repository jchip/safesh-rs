mod common;
use common::*;

#[test]
fn check_allows_pipeline() {
    let e = Env::new();
    let r = e.run_safesh(&["check", "ls | wc -l"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(
        r.stdout.lines().last(),
        Some("decision: allow"),
        "{}",
        r.stdout
    );
}

#[test]
fn check_asks_for_unknown_command() {
    let e = Env::new();
    let r = e.run_safesh(&["check", "foobarbaz --version"]);
    assert_eq!(
        r.stdout.lines().last(),
        Some("decision: ask"),
        "{}",
        r.stdout
    );
}

#[test]
fn rewrite_then_run() {
    let e = Env::new();
    let h = e.hook("echo hi > out.txt && cat out.txt");
    assert_eq!(h.decision(), "allow", "{} {}", h.stdout, h.stderr);
    let r = e.sh(&h.command());
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(r.stdout.trim(), "hi");
    assert_eq!(e.read_proj("out.txt").trim(), "hi");
}

#[test]
fn allowed_pipeline_runs() {
    let e = Env::new();
    let r = e.hook_run("printf 'a\\nb\\n' | wc -l");
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(r.stdout.trim(), "2");
}

#[test]
fn redirect_outside_blocked_at_pre_check() {
    let e = Env::new();
    let t = "/etc/safesh-e2e-redirect";
    let h = e.hook(&format!("echo x > {t}"));
    assert_ne!(h.decision(), "allow", "{}", h.stdout);
    assert!(h.reason().contains("PATH BLOCKED"), "{}", h.reason());
    assert!(h.command().is_empty());
    assert_absent(t);
}

#[test]
fn cd_trailer_present_and_correct() {
    let e = Env::new();
    std::fs::create_dir(e.proj.join("sub")).unwrap();
    let h = e.hook("cd sub && pwd");
    let c = h.command();
    let want = format!("&& cd '{}'", e.proj.join("sub").display());
    assert!(c.ends_with(&want), "{c}");
    let r = e.sh(&c);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(r.stdout.trim(), e.proj.join("sub").to_string_lossy());
}

#[test]
fn control_plane_passes_through() {
    let e = Env::new();
    let h = e.hook("safesh retry --id=1-2 --choice=1");
    assert_eq!(h.code, 0);
    assert_eq!(h.stdout, "");
}

#[test]
fn invalid_json_exits_2() {
    let e = Env::new();
    let h = e.hook_raw(&["hook", "claude"], "not json{");
    assert_eq!(h.code, 2);
    assert_eq!(h.stdout, "");
}

#[test]
fn non_bash_tool_passes_through() {
    let e = Env::new();
    let input = r#"{"tool_name":"Read","tool_input":{"file_path":"/etc/hosts"},"session_id":"s1"}"#;
    let h = e.hook_raw(&["hook", "claude"], input);
    assert_eq!(h.code, 0);
    assert_eq!(h.stdout, "");
}

#[test]
fn shadow_passes_through_and_logs() {
    let e = Env::new();
    let h = e.hook_raw(&["hook", "claude", "--shadow"], &e.input("echo hi", "s1"));
    assert_eq!((h.code, h.stdout.as_str()), (0, ""));
    let log = std::fs::read_to_string(e.state.join("shadow.jsonl")).expect("shadow.jsonl");
    let line: serde_json::Value = serde_json::from_str(log.lines().next().unwrap()).unwrap();
    assert_eq!(line["decision"], "rewrite");
    assert_eq!(line["command"], "echo hi");
    let h = e.hook_raw(&["hook", "claude", "--shadow"], &e.input("foobarbaz", "s1"));
    assert_eq!(h.stdout, "");
    let log = std::fs::read_to_string(e.state.join("shadow.jsonl")).unwrap();
    assert_eq!(log.lines().count(), 2);
    assert!(log.lines().nth(1).unwrap().contains("\"deny\""));
}

#[test]
fn codex_rewrite_approval_consumed_once() {
    let e = Env::new();
    let h = e.hook_as("codex", "echo hi", "s1");
    let rewritten = h.command();
    assert!(!rewritten.is_empty(), "{} {}", h.stdout, h.stderr);
    let approvals = || std::fs::read_dir(e.state.join("approvals")).map_or(0, Iterator::count);
    assert_eq!(approvals(), 1);
    let pr = serde_json::json!({
        "hook_event_name": "PermissionRequest",
        "tool_name": "Bash",
        "tool_input": {"command": rewritten},
        "cwd": e.proj,
        "session_id": "s1",
    })
    .to_string();
    let first = e.hook_raw(&["hook", "codex"], &pr);
    assert_eq!(first.code, 0, "{}", first.stderr);
    assert!(first.stdout.contains("allow"), "{}", first.stdout);
    assert_eq!(approvals(), 0);
    let second = e.hook_raw(&["hook", "codex"], &pr);
    assert_eq!(second.stdout, "", "approval was reused");
}

#[test]
fn unmatched_permission_request_is_silent() {
    let e = Env::new();
    let pr = serde_json::json!({
        "hook_event_name": "PermissionRequest",
        "tool_name": "Bash",
        "tool_input": {"command": "rm -rf x"},
        "cwd": e.proj,
        "session_id": "s1",
    })
    .to_string();
    let h = e.hook_raw(&["hook", "codex"], &pr);
    assert_eq!((h.code, h.stdout.as_str()), (0, ""));
}

#[test]
fn rm_and_mv_inside_project_allowed() {
    let e = Env::new();
    e.write("junk", "x");
    e.write("a", "x");
    assert_eq!(e.hook("rm junk").decision(), "allow");
    assert_eq!(e.hook("mv a b").decision(), "allow");
    let r = e.hook_run("mv a b && rm junk");
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(e.proj.join("b").exists());
    assert!(!e.proj.join("junk").exists());
}

#[test]
fn mv_to_outside_asks() {
    let e = Env::new();
    e.write("f", "x");
    let t = "/etc/safesh-e2e-mv";
    let h = e.hook(&format!("mv f {t}"));
    assert_ne!(h.decision(), "allow", "{}", h.stdout);
    assert!(h.reason().contains("BLOCKED"), "{}", h.reason());
    assert!(e.proj.join("f").exists());
    assert_absent(t);
}

#[test]
fn nested_shell_with_env_asks_at_pre_check() {
    let e = Env::new();
    let t = "/etc/safesh-e2e-envbash";
    let h = e.hook(&format!("env bash -c 'touch {t}'"));
    assert_ne!(h.decision(), "allow", "{}", h.stdout);
    assert!(h.reason().contains("BLOCKED"), "{}", h.reason());
    assert_absent(t);
}

#[test]
fn xargs_rm_asks() {
    let e = Env::new();
    e.write("f", "x");
    let h = e.hook("echo f | xargs rm");
    assert_ne!(h.decision(), "allow", "{}", h.stdout);
    assert!(h.reason().contains("BLOCKED"), "{}", h.reason());
    assert!(e.proj.join("f").exists());
}

#[test]
fn find_exec_rm_follows_the_search_root() {
    // `{}` counts as a path under the search root, so inside the project it is allowed,
    // like `rm` there; a root outside the allow lists asks.
    let e = Env::new();
    e.write("f", "x");
    let inside = e.hook(r"find . -name f -exec rm {} \;");
    assert_eq!(inside.decision(), "allow", "{}", inside.stdout);
    let outside = e.hook(r"find /etc -name nothing-here -exec rm {} \;");
    assert_ne!(outside.decision(), "allow", "{}", outside.stdout);
    assert!(outside.reason().contains("BLOCKED"), "{}", outside.reason());
}

#[test]
fn rule_ask_for_git_push_force() {
    let e = Env::new();
    e.write_global_config(r#"{"rules":[{"match":["git","push","--force","**"],"action":"ask"}]}"#);
    let h = e.hook("git push --force origin main");
    assert_ne!(h.decision(), "allow", "{}", h.stdout);
    assert!(h.reason().contains("BLOCKED"), "{}", h.reason());
    let ok = e.hook("git status");
    assert_eq!(ok.decision(), "allow", "{}", ok.stdout);
}

#[test]
fn install_claude_dry_run_prints_and_writes_nothing() {
    let e = Env::new();
    let r = e.run_safesh(&["install", "claude", "--dry-run"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stdout.contains("would write"), "{}", r.stdout);
    assert!(r.stdout.contains(" hook claude"), "{}", r.stdout);
    assert!(r.stdout.contains("--budget-sec 590"), "{}", r.stdout);
    assert!(r.stdout.contains("\"timeout\": 600"), "{}", r.stdout);
    assert!(r.stdout.contains("PreToolUse"));
    assert!(!e.home.join(".claude/settings.json").exists());
}

#[test]
fn install_claude_shadow_next_to_ts_hook() {
    let e = Env::new();
    let p = e.home.join(".claude/settings.json");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    let ts = r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"deno run hooks/bash-prehook.ts"}]}]}}"#;
    std::fs::write(&p, ts).unwrap();
    let r = e.run_safesh(&["install", "claude"]);
    assert_eq!(r.code, 1, "{}", r.stdout);
    assert_eq!(std::fs::read_to_string(&p).unwrap(), ts);
    let r = e.run_safesh(&["install", "claude-shadow"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
    let pre = v["hooks"]["PreToolUse"].as_array().unwrap();
    assert_eq!(pre.len(), 2);
    assert!(
        pre[0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .contains("bash-prehook")
    );
    assert!(
        pre[1]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .ends_with("hook claude --shadow --budget-sec 590")
    );
    assert_eq!(pre[1]["hooks"][0]["timeout"], 600);
}

#[test]
fn find_exec_outside_target_asks() {
    let e = Env::new();
    e.write("f", "x");
    let t = "/etc/safesh-e2e-find";
    let h = e.hook(&format!(r"find . -name f -exec touch {t} \;"));
    assert_ne!(h.decision(), "allow", "{}", h.stdout);
    assert_absent(t);
}

#[test]
fn agent_cannot_write_its_own_grants() {
    // Config and session files hold grants; a redirect into them is denied outright.
    let e = Env::new();
    let local = e.proj.join(".config/safesh/config.local.json");
    let h = e.hook(r#"mkdir -p .config/safesh && echo '{"allowedCommands":["evil"]}' > .config/safesh/config.local.json"#);
    assert_eq!(h.decision(), "deny", "{}", h.stdout);
    assert!(h.reason().contains("DENIED"), "{}", h.reason());
    assert!(!local.exists());
    let h = e.hook("echo '{}' > .temp/safesh/session-s1.json");
    assert_eq!(h.decision(), "deny", "{}", h.stdout);
}

#[test]
fn allow_rule_does_not_cover_another_program_with_that_name() {
    // D2: an allow rule for `git status` is about the `git` that PATH finds, not any file named git.
    let e = Env::new();
    let fake = fake_tool(&e.outside.join("bin"), "git");
    e.write_global_config(r#"{"rules":[{"match":["git","status"],"action":"allow"}]}"#);
    let h = e.hook(&format!("'{}' status", fake.display()));
    assert_ne!(h.decision(), "allow", "{}", h.stdout);
    assert!(h.reason().contains("BLOCKED"), "{}", h.reason());
    let ok = e.hook("git status");
    assert_eq!(ok.decision(), "allow", "{}", ok.stdout);
}

#[test]
fn allow_rule_that_names_the_path_covers_it() {
    let e = Env::new();
    let fake = fake_tool(&e.outside.join("bin"), "deploy");
    e.write_global_config(
        &serde_json::json!({"rules": [{"match": [fake, "--dry-run"], "action": "allow"}]})
            .to_string(),
    );
    let h = e.hook(&format!("'{}' --dry-run", fake.display()));
    assert_eq!(h.decision(), "allow", "{}", h.stdout);
    let other = e.hook(&format!("'{}' --for-real", fake.display()));
    assert_ne!(other.decision(), "allow", "{}", other.stdout);
}
