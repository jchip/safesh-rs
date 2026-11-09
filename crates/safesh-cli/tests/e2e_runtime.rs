mod common;
use common::*;

/// Hook must allow (the name is only known at runtime), then the rewrite must stop with 126
/// before `after`, and nothing may be created at `target`.
fn runtime_denied(script: &str, target: &std::path::Path, not_printed: &str) -> Run {
    let e = Env::new();
    let r = e.hook_run(script);
    assert_eq!(r.code, 126, "stdout={} stderr={}", r.stdout, r.stderr);
    assert!(
        !r.stdout.contains(not_printed),
        "ran past denial: {}",
        r.stdout
    );
    assert!(r.stderr.contains("[SAFESH]"), "{}", r.stderr);
    assert_absent(target);
    r
}

#[test]
fn runtime_variable_command_name_denied() {
    let (t, th) = denied_target("safesh-e2e-var");
    runtime_denied(
        &format!("x=$(echo touch); $x {t}; echo after"),
        &th,
        "after",
    );
}

#[test]
fn runtime_denial_in_pipeline_stops_run() {
    let (t, th) = denied_target("safesh-e2e-pipe");
    let r = runtime_denied(
        &format!("x=$(echo touch); echo before; $x {t} | cat; echo after"),
        &th,
        "after",
    );
    assert!(r.stdout.contains("before"), "{}", r.stdout);
}

#[test]
fn runtime_denial_in_for_loop() {
    let (t, th) = denied_target("safesh-e2e-for");
    runtime_denied(
        &format!("x=$(echo touch); for i in 1 2; do $x {t}$i; echo after; done"),
        &th,
        "after",
    );
    assert_absent(format!("{}1", th.display()));
    assert_absent(format!("{}2", th.display()));
}

#[test]
fn runtime_denial_in_function() {
    let (t, th) = denied_target("safesh-e2e-fn");
    runtime_denied(
        &format!("x=$(echo touch); f() {{ $x {t}; echo inside; }}; f; echo after"),
        &th,
        "inside",
    );
}

#[test]
fn runtime_denial_in_eval() {
    let (t, th) = denied_target("safesh-e2e-eval");
    runtime_denied(
        &format!("x=$(echo touch); eval \"$x {t}\"; echo after"),
        &th,
        "after",
    );
}

#[test]
fn nested_bash_c_denied_stops_parent() {
    let (t, th) = denied_target("safesh-e2e-bashc");
    let e = Env::new();
    let h = e.hook(&format!("bash -c 'touch {t}'; echo after"));
    assert_eq!(h.decision(), "allow", "{}", h.stdout);
    let r = e.sh(&h.command());
    assert_eq!(r.code, 126, "stdout={} stderr={}", r.stdout, r.stderr);
    assert!(!r.stdout.contains("after"), "{}", r.stdout);
    assert_absent(&th);
}

#[test]
fn sh_script_denied_stops_parent() {
    let (t, th) = denied_target("safesh-e2e-shscript");
    let e = Env::new();
    e.write("s.sh", &format!("touch {t}\n"));
    let h = e.hook("sh s.sh; echo after");
    assert_eq!(h.decision(), "allow", "{}", h.stdout);
    let r = e.sh(&h.command());
    assert_eq!(r.code, 126, "stdout={} stderr={}", r.stdout, r.stderr);
    assert!(!r.stdout.contains("after"), "{}", r.stdout);
    assert_absent(&th);
}

#[test]
fn nested_bash_inside_project_still_runs() {
    let e = Env::new();
    let r = e.hook_run("bash -c 'echo nested-ok > n.txt'; cat n.txt");
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stdout.contains("nested-ok"), "{}", r.stdout);
}

#[test]
fn runtime_write_to_session_grants_is_denied() {
    let e = Env::new();
    std::fs::create_dir_all(e.proj.join(".temp/safesh")).unwrap();
    let r = e.hook_run(r#"x=$(echo tee); echo '{"allowedCommands":["evil"]}' | $x .temp/safesh/session-s1.json; echo after"#);
    assert_eq!(r.code, 126, "stdout={} stderr={}", r.stdout, r.stderr);
    assert!(r.stderr.contains("DENIED"), "{}", r.stderr);
    assert!(!r.stdout.contains("after"), "{}", r.stdout);
    assert!(!e.proj.join(".temp/safesh/session-s1.json").exists());
}

#[test]
#[cfg_attr(
    windows,
    ignore = "the fake tool is a #! script, which does not start on Windows"
)]
fn allow_rule_does_not_cover_a_hidden_program_path() {
    // The pre-check can't see the path, so only the runtime check stands between it and the run.
    let e = Env::new();
    let fake = fake_tool(&e.outside.join("bin"), "git");
    e.write_global_config(r#"{"rules":[{"match":["git","status"],"action":"allow"}]}"#);
    let r = e.hook_run(&format!(
        "g=$(printf %s '{}'); \"$g\" status; echo after",
        fake.display()
    ));
    assert_eq!(r.code, 126, "stdout={} stderr={}", r.stdout, r.stderr);
    assert!(!r.stdout.contains("RAN git"), "fake ran: {}", r.stdout);
    assert!(!r.stdout.contains("after"), "ran past denial: {}", r.stdout);
}

#[test]
fn allow_rule_does_not_follow_a_path_the_script_changed() {
    // The script puts the fake first on PATH. The rule's `git` is still the one the
    // session's PATH finds.
    let e = Env::new();
    let bin = e.outside.join("bin");
    // Windows finds a PATH program only with a `PATHEXT` extension, so the fake must be
    // `git.exe` to come first. It is a `#!` script and can't start, which is fine: the gate must
    // stop it before exec.
    fake_tool(&bin, if cfg!(windows) { "git.exe" } else { "git" });
    e.write_global_config(r#"{"rules":[{"match":["git","status"],"action":"allow"}]}"#);
    // Under brush on Windows `$PATH` is the native `;` list, not Git Bash's `:` form.
    let sep = if cfg!(windows) { ';' } else { ':' };
    let script = format!(
        "PATH=\"{}{sep}$PATH\"; git status; echo after",
        sh_word(&bin)
    );
    let h = e.hook(&script);
    if h.decision() == "allow" {
        let r = e.sh(&h.command());
        assert_eq!(r.code, 126, "stdout={} stderr={}", r.stdout, r.stderr);
        // A failed exec is 126 too; the denial must come from the gate.
        assert!(r.stderr.contains("[SAFESH]"), "{}", r.stderr);
        assert!(!r.stdout.contains("RAN git"), "fake ran: {}", r.stdout);
    }
}

#[test]
#[cfg_attr(
    windows,
    ignore = "the fake tool is a #! script, which does not start on Windows"
)]
fn allow_rule_still_runs_the_program_path_resolves() {
    // A tool on no allow list, allowed only by the rule, resolved through PATH.
    let td = tempfile::Builder::new()
        .prefix("rulebin")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap();
    let bin = td.path().canonicalize().unwrap();
    fake_tool(&bin, "safesh-rule-tool");
    let e = Env::new().with_path_prefix(bin.clone());
    e.write_global_config(r#"{"rules":[{"match":["safesh-rule-tool","go"],"action":"allow"}]}"#);
    let r = e.hook_run("safesh-rule-tool go");
    assert_eq!(r.code, 0, "stdout={} stderr={}", r.stdout, r.stderr);
    assert!(r.stdout.contains("RAN safesh-rule-tool"), "{}", r.stdout);
    let by_path = e.hook_run(&format!("'{}' go", bin.join("safesh-rule-tool").display()));
    assert_eq!(
        by_path.code, 0,
        "stdout={} stderr={}",
        by_path.stdout, by_path.stderr
    );
    let without_rule = e.hook("safesh-rule-tool stop");
    assert_ne!(without_rule.decision(), "allow", "{}", without_rule.stdout);
}
