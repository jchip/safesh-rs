mod common;
use common::*;

#[test]
fn unknown_command_blocked_then_retry_once_runs() {
    let e = Env::new();
    let h = e.hook("foobarbaz --version; echo after");
    assert_ne!(h.decision(), "allow", "{}", h.stdout);
    assert!(h.reason().contains("BLOCKED: foobarbaz"), "{}", h.reason());
    let r = e.run_safesh(&["retry", &format!("--id={}", h.pending_id()), "--choice=1"]);
    // The command does not exist, so it fails to run, but the script itself is let through.
    assert!(
        r.stdout.contains("after"),
        "stdout={} stderr={}",
        r.stdout,
        r.stderr
    );
}

#[test]
fn retry_deny_runs_nothing() {
    let e = Env::new();
    let h = e.hook("foobarbaz; echo after > ran.txt");
    let id = h.pending_id();
    let r = e.run_safesh(&["retry", &format!("--id={id}"), "--choice=4"]);
    assert_eq!(r.code, 0);
    assert!(r.stderr.contains("denied"), "{}", r.stderr);
    assert!(!e.proj.join("ran.txt").exists());
    let again = e.run_safesh(&["retry", &format!("--id={id}"), "--choice=1"]);
    assert_ne!(again.code, 0, "pending id should be gone");
}

#[test]
fn retry_always_persists_and_next_hook_allows() {
    let e = Env::new();
    let h = e.hook("foobarbaz --version; echo after");
    let r = e.run_safesh(&["retry", &format!("--id={}", h.pending_id()), "--choice=2"]);
    let cfg = e.proj.join(".config/safesh/config.local.json");
    let text = std::fs::read_to_string(&cfg).unwrap_or_else(|_| panic!("no config: {}", r.stderr));
    assert!(text.contains("foobarbaz"), "{text}");
    let h2 = e.hook("foobarbaz --version; echo after");
    assert_eq!(h2.decision(), "allow", "{}", h2.stdout);
}

#[test]
fn retry_session_scope_applies_to_same_session_only() {
    let e = Env::new();
    let h = e.hook("foobarbaz");
    e.run_safesh(&["retry", &format!("--id={}", h.pending_id()), "--choice=3"]);
    assert_eq!(e.hook("foobarbaz").decision(), "allow");
    assert_ne!(e.hook_as("claude", "foobarbaz", "s2").decision(), "allow");
}

#[test]
fn retry_always_global_persists_to_global_config() {
    let e = Env::new();
    let h = e.hook("foobarbaz");
    e.run_safesh(&["retry", &format!("--id={}", h.pending_id()), "--choice=5"]);
    let g = std::fs::read_to_string(e.home.join(".config/safesh/config.json")).unwrap();
    assert!(g.contains("foobarbaz"), "{g}");
    assert_eq!(e.hook("foobarbaz").decision(), "allow");
}

#[test]
fn retry_rejects_bad_choice() {
    let e = Env::new();
    let h = e.hook("foobarbaz");
    let r = e.run_safesh(&["retry", &format!("--id={}", h.pending_id()), "--choice=9"]);
    assert_eq!(r.code, 1);
}

/// A write outside the project but inside the temp root: blocked, then granted by retry-path.
fn outside_write(e: &Env, name: &str) -> (String, std::path::PathBuf) {
    let dir = e.root.join("outside");
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join(name);
    (format!("echo data > {}", f.display()), f)
}

#[test]
fn retry_path_write_once() {
    let e = Env::new();
    let (cmd, f) = outside_write(&e, "w1.txt");
    let h = e.hook(&cmd);
    assert!(
        h.reason().contains("PATH BLOCKED"),
        "{} {}",
        h.stdout,
        h.stderr
    );
    assert!(!f.exists());
    let r = e.run_safesh(&[
        "retry-path",
        &format!("--id={}", h.pending_id()),
        "--choice=w1",
    ]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(f.exists());
    assert!(
        e.hook(&cmd).reason().contains("PATH BLOCKED"),
        "once must not persist"
    );
}

#[test]
fn retry_path_write_session() {
    let e = Env::new();
    let (cmd, f) = outside_write(&e, "w2.txt");
    let h = e.hook(&cmd);
    let r = e.run_safesh(&[
        "retry-path",
        &format!("--id={}", h.pending_id()),
        "--choice=w2",
    ]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(f.exists());
    assert_eq!(e.hook(&cmd).decision(), "allow");
    assert_ne!(e.hook_as("claude", &cmd, "s2").decision(), "allow");
}

#[test]
fn retry_path_deny_leaves_nothing() {
    let e = Env::new();
    let t = "/etc/safesh-e2e-pathdeny";
    let h = e.hook(&format!("echo x > {t}"));
    assert!(h.reason().contains("PATH BLOCKED"), "{}", h.reason());
    let r = e.run_safesh(&[
        "retry-path",
        &format!("--id={}", h.pending_id()),
        "--choice=4",
    ]);
    assert_eq!(r.code, 0);
    assert!(r.stderr.contains("denied"), "{}", r.stderr);
    assert_absent(t);
}

#[test]
fn retry_path_bad_choice_fails() {
    let e = Env::new();
    let (cmd, f) = outside_write(&e, "bad.txt");
    let h = e.hook(&cmd);
    let r = e.run_safesh(&[
        "retry-path",
        &format!("--id={}", h.pending_id()),
        "--choice=zz",
    ]);
    assert_eq!(r.code, 1);
    assert!(!f.exists());
}

#[test]
fn runtime_block_offers_retry_that_resumes() {
    let e = Env::new();
    let dir = e.root.join("outside");
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("rt.txt");
    let script = format!("x=$(echo touch); $x {}; echo after", f.display());
    let h = e.hook(&script);
    assert_eq!(h.decision(), "allow", "{}", h.stdout);
    let r = e.sh(&h.command());
    assert_eq!(r.code, 126, "{}", r.stderr);
    assert!(!f.exists());
    let id = pending_id(&r.stderr);
    let r = e.run_safesh(&["retry-path", &format!("--id={id}"), "--choice=w1"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(f.exists());
    assert!(r.stdout.contains("after"));
}
