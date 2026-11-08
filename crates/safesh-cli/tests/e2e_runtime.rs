mod common;
use common::*;

/// Hook must allow (the name is only known at runtime), then the rewrite must stop with 126
/// before `after`, and nothing may be created at `target`.
fn runtime_denied(script: &str, target: &str, not_printed: &str) -> Run {
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
    let t = "/etc/safesh-e2e-var";
    runtime_denied(&format!("x=$(echo touch); $x {t}; echo after"), t, "after");
}

#[test]
fn runtime_denial_in_pipeline_stops_run() {
    let t = "/etc/safesh-e2e-pipe";
    let r = runtime_denied(
        &format!("x=$(echo touch); echo before; $x {t} | cat; echo after"),
        t,
        "after",
    );
    assert!(r.stdout.contains("before"), "{}", r.stdout);
}

#[test]
fn runtime_denial_in_for_loop() {
    let t = "/etc/safesh-e2e-for";
    runtime_denied(
        &format!("x=$(echo touch); for i in 1 2; do $x {t}$i; echo after; done"),
        t,
        "after",
    );
    assert_absent(format!("{t}1"));
    assert_absent(format!("{t}2"));
}

#[test]
fn runtime_denial_in_function() {
    let t = "/etc/safesh-e2e-fn";
    runtime_denied(
        &format!("x=$(echo touch); f() {{ $x {t}; echo inside; }}; f; echo after"),
        t,
        "inside",
    );
}

#[test]
fn runtime_denial_in_eval() {
    let t = "/etc/safesh-e2e-eval";
    runtime_denied(
        &format!("x=$(echo touch); eval \"$x {t}\"; echo after"),
        t,
        "after",
    );
}

#[test]
fn nested_bash_c_denied_stops_parent() {
    let t = "/etc/safesh-e2e-bashc";
    let e = Env::new();
    let h = e.hook(&format!("bash -c 'touch {t}'; echo after"));
    assert_eq!(h.decision(), "allow", "{}", h.stdout);
    let r = e.sh(&h.command());
    assert_eq!(r.code, 126, "stdout={} stderr={}", r.stdout, r.stderr);
    assert!(!r.stdout.contains("after"), "{}", r.stdout);
    assert_absent(t);
}

#[test]
fn sh_script_denied_stops_parent() {
    let t = "/etc/safesh-e2e-shscript";
    let e = Env::new();
    e.write("s.sh", &format!("touch {t}\n"));
    let h = e.hook("sh s.sh; echo after");
    assert_eq!(h.decision(), "allow", "{}", h.stdout);
    let r = e.sh(&h.command());
    assert_eq!(r.code, 126, "stdout={} stderr={}", r.stdout, r.stderr);
    assert!(!r.stdout.contains("after"), "{}", r.stdout);
    assert_absent(t);
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
    fake_tool(&bin, "git");
    e.write_global_config(r#"{"rules":[{"match":["git","status"],"action":"allow"}]}"#);
    let script = format!("PATH='{}':\"$PATH\"; git status; echo after", bin.display());
    let h = e.hook(&script);
    if h.decision() == "allow" {
        let r = e.sh(&h.command());
        assert_eq!(r.code, 126, "stdout={} stderr={}", r.stdout, r.stderr);
        assert!(!r.stdout.contains("RAN git"), "fake ran: {}", r.stdout);
    }
}

#[test]
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
