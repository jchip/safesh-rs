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
