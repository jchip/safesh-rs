//! `.sh` scripts: they run, and the commands inside them are still checked. This holds for
//! `bash s.sh`, `source`, and `./s.sh` through its `#!` line when the script is in the project.

mod common;
use common::*;

/// Writes an executable script in the project.
fn script(e: &Env, rel: &str, body: &str) {
    e.write(rel, body);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(e.proj.join(rel), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// An env whose PATH finds `deploy`, a real program on no allow list.
fn env_with_deploy() -> Env {
    let e = Env::new();
    let bin = e.outside.join("bin");
    fake_tool(&bin, "deploy");
    e.with_path_prefix(bin)
}

/// The run stopped with 126 at the denial, before `in2` and `after`.
fn assert_stopped(r: &Run) {
    assert_eq!(r.code, 126, "stdout={} stderr={}", r.stdout, r.stderr);
    assert!(r.stderr.contains("[SAFESH]"), "{}", r.stderr);
    assert!(!r.stdout.contains("in2"), "ran past denial: {}", r.stdout);
    assert!(!r.stdout.contains("after"), "ran past denial: {}", r.stdout);
}

#[test]
fn bash_script_runs_with_args() {
    let e = Env::new();
    script(&e, "s.sh", "echo \"arg=$1\" > out.txt; cat out.txt\n");
    let r = e.hook_run("bash s.sh hello; echo after");
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(r.stdout, "arg=hello\nafter\n");
}

#[test]
fn source_denied_write_stops_run() {
    let (t, th) = denied_target("safesh-e2e-source");
    let e = Env::new();
    script(&e, "s.sh", &format!("touch {t}\necho in2\n"));
    assert_stopped(&e.hook_run("source ./s.sh; echo after"));
    assert_absent(&th);
}

#[test]
fn bash_script_hidden_command_blocked() {
    let e = env_with_deploy();
    script(&e, "s.sh", "x=$(echo deploy); $x; echo in2\n");
    let r = e.hook_run("bash s.sh; echo after");
    assert_stopped(&r);
    assert!(!r.stdout.contains("RAN deploy"), "{}", r.stdout);
}

#[test]
#[cfg_attr(windows, ignore = "#! scripts do not start on Windows")]
fn direct_script_runs_with_args() {
    let e = Env::new();
    script(
        &e,
        "s.sh",
        "#!/bin/sh\necho \"$0 arg=$1\" > out.txt; cat out.txt\n",
    );
    let r = e.hook_run("./s.sh hello; echo after");
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(r.stdout, "./s.sh arg=hello\nafter\n");
}

#[test]
#[cfg_attr(windows, ignore = "#! scripts do not start on Windows")]
fn env_bash_script_runs_bash_syntax() {
    let e = Env::new();
    script(
        &e,
        "s.sh",
        "#!/usr/bin/env bash\na=(x y); [[ ${#a[@]} == 2 ]] && echo ok\n",
    );
    let r = e.hook_run("./s.sh");
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(r.stdout, "ok\n");
}

#[test]
#[cfg_attr(windows, ignore = "#! scripts do not start on Windows")]
fn shebang_options_apply() {
    let e = Env::new();
    script(&e, "s.sh", "#!/bin/sh -e\nfalse\necho in2\n");
    let r = e.hook_run("./s.sh; echo after");
    assert_eq!(r.stdout, "after\n", "{}", r.stderr);
}

#[test]
#[cfg_attr(windows, ignore = "#! scripts do not start on Windows")]
fn direct_script_denied_write_stops_run() {
    let (t, th) = denied_target("safesh-e2e-direct");
    let e = Env::new();
    script(
        &e,
        "s.sh",
        &format!("#!/bin/sh\necho in; touch {t}; echo in2\n"),
    );
    let r = e.hook_run("./s.sh; echo after");
    assert_stopped(&r);
    assert!(r.stdout.contains("in"), "{}", r.stdout);
    assert_absent(&th);
}

#[test]
#[cfg_attr(windows, ignore = "#! scripts do not start on Windows")]
fn direct_script_hidden_command_blocked() {
    let e = env_with_deploy();
    script(&e, "s.sh", "#!/bin/sh\nx=$(echo deploy); $x; echo in2\n");
    let r = e.hook_run("./s.sh; echo after");
    assert_stopped(&r);
    assert!(!r.stdout.contains("RAN deploy"), "{}", r.stdout);
}

#[test]
#[cfg_attr(windows, ignore = "#! scripts do not start on Windows")]
fn env_bash_script_denied_write_stops_run() {
    let (t, th) = denied_target("safesh-e2e-envbash");
    let e = Env::new();
    script(
        &e,
        "s.sh",
        &format!("#!/usr/bin/env bash\ntouch {t}\necho in2\n"),
    );
    assert_stopped(&e.hook_run("./s.sh; echo after"));
    assert_absent(&th);
}

#[test]
#[cfg_attr(windows, ignore = "#! scripts do not start on Windows")]
fn script_calling_script_is_checked() {
    let (t, th) = denied_target("safesh-e2e-chain");
    let e = Env::new();
    script(&e, "s.sh", "#!/bin/sh\n./bin/t.sh\necho in2\n");
    script(&e, "bin/t.sh", &format!("#!/bin/sh\ntouch {t}\n"));
    assert_stopped(&e.hook_run("./s.sh; echo after"));
    assert_absent(&th);
}

#[test]
#[cfg_attr(
    windows,
    ignore = "the fake tool is a #! script, which does not start on Windows"
)]
fn shell_script_tool_outside_project_runs_natively() {
    // Allowed tools on PATH that are shell scripts (a git wrapper, brew) keep running as
    // before. Only agent scripts run in the engine.
    let td = tempfile::Builder::new()
        .prefix("rulebin")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap();
    let bin = td.path().canonicalize().unwrap();
    fake_tool(&bin, "deploy");
    std::fs::write(bin.join("safesh-wrap"), "#!/bin/sh\nx=$(echo deploy); $x\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            bin.join("safesh-wrap"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }
    let e = Env::new().with_path_prefix(bin);
    e.write_global_config(r#"{"rules":[{"match":["safesh-wrap"],"action":"allow"}]}"#);
    let r = e.hook_run("safesh-wrap");
    assert_eq!(r.code, 0, "stdout={} stderr={}", r.stdout, r.stderr);
    assert!(r.stdout.contains("RAN deploy"), "{}", r.stdout);
}
