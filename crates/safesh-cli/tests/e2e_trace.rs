mod common;
use common::*;
use serde_json::Value;

/// All events logged so far. Reads whatever day files exist, so it does not depend on the date.
fn events(e: &Env) -> Vec<Value> {
    let dir = e.state.join("log");
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return out;
    };
    let mut files: Vec<_> = rd.flatten().map(|f| f.path()).collect();
    files.sort();
    for f in files {
        for line in std::fs::read_to_string(f).unwrap().lines() {
            out.push(serde_json::from_str(line).expect("log line is JSON"));
        }
    }
    out
}

fn find<'a>(evs: &'a [Value], ev: &str) -> Vec<&'a Value> {
    evs.iter().filter(|v| v["ev"] == ev).collect()
}

fn active_files(e: &Env) -> Vec<std::path::PathBuf> {
    match std::fs::read_dir(e.state.join("active")) {
        Ok(rd) => rd.flatten().map(|f| f.path()).collect(),
        Err(_) => Vec::new(),
    }
}

fn write_active(e: &Env, pid: u32, kind: &str, id: &str, step: &str) -> std::path::PathBuf {
    let dir = e.state.join("active");
    std::fs::create_dir_all(&dir).unwrap();
    let ts = "2026-01-01T00:00:00.000+00:00";
    let p = dir.join(format!("{pid}.json"));
    let body = serde_json::json!({
        "pid": pid, "kind": kind, "id": id, "session": "s1", "cwd": e.proj,
        "started": ts, "step": step, "step_at": ts,
    });
    std::fs::write(&p, body.to_string()).unwrap();
    p
}

#[test]
fn hook_and_run_are_logged() {
    let e = Env::new();
    let r = e.hook_run("echo hi");
    assert_eq!(r.code, 0, "{}", r.stderr);
    let evs = events(&e);
    assert_eq!(find(&evs, "hook.start").len(), 1, "{evs:?}");
    let input = find(&evs, "hook.input");
    assert!(input[0]["cmd"].as_str().unwrap().contains("echo hi"));
    let dec = find(&evs, "hook.decision");
    assert_eq!(dec[0]["decision"], "rewrite");
    let id = dec[0]["id"].as_str().expect("decision has id");
    assert_eq!(find(&evs, "hook.end")[0]["code"], 0);
    assert_eq!(find(&evs, "run.start")[0]["id"], id);
    let end = find(&evs, "run.end");
    assert_eq!(end[0]["id"], id);
    assert_eq!(end[0]["code"], 0);
    assert!(active_files(&e).is_empty(), "{:?}", active_files(&e));
}

#[cfg(unix)]
#[test]
fn runtime_denial_is_logged() {
    let e = Env::new();
    let (t, _) = denied_target("safesh-e2e-trace");
    let r = e.hook_run(&format!("x=$(echo touch); $x {t}"));
    assert_eq!(r.code, 126, "stdout={} stderr={}", r.stdout, r.stderr);
    let evs = events(&e);
    let id = find(&evs, "run.start")[0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let blocked = find(&evs, "exec.blocked");
    assert!(!blocked.is_empty(), "{evs:?}");
    assert_eq!(blocked[0]["id"], id.as_str());
    let end = find(&evs, "run.end");
    assert_eq!(end[0]["code"], 126, "{evs:?}");
    assert!(active_files(&e).is_empty(), "{:?}", active_files(&e));
}

#[test]
fn status_empty() {
    let e = Env::new();
    let r = e.run_safesh(&["status"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stdout.contains("nothing in progress"), "{}", r.stdout);
}

#[cfg(unix)]
#[test]
fn status_reports_and_removes_dead_entries() {
    let e = Env::new();
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let pid = child.id();
    child.wait().unwrap();
    let f = write_active(&e, pid, "run", "dead01", "exec sleep 9");
    let r = e.run_safesh(&["status"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stdout.contains("gone without finishing"), "{}", r.stdout);
    assert!(r.stdout.contains("exec sleep 9"), "{}", r.stdout);
    assert!(!f.exists());
    let r2 = e.run_safesh(&["status"]);
    assert!(r2.stdout.contains("nothing in progress"), "{}", r2.stdout);
}

#[test]
fn status_shows_live_entry() {
    let e = Env::new();
    let f = write_active(&e, std::process::id(), "run", "abc123", "exec cargo test");
    let r = e.run_safesh(&["status"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stdout.contains("PID"), "{}", r.stdout);
    assert!(r.stdout.contains("abc123"), "{}", r.stdout);
    assert!(r.stdout.contains("exec cargo test"), "{}", r.stdout);
    assert!(f.exists());
}

#[test]
fn log_filters_by_id() {
    let e = Env::new();
    assert_eq!(e.hook_run("echo one").code, 0);
    assert_eq!(e.hook_run("echo two").code, 0);
    let ids: Vec<String> = find(&events(&e), "hook.decision")
        .iter()
        .map(|v| v["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(ids.len(), 2);
    let r = e.run_safesh(&["log", "--id", &ids[0][..6]]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stdout.contains(&ids[0]), "{}", r.stdout);
    assert!(!r.stdout.contains(&ids[1]), "{}", r.stdout);
    let one = e.run_safesh(&["log", "-n", "1"]);
    assert_eq!(one.stdout.lines().count(), 1, "{}", one.stdout);
}

#[test]
fn log_missing_day() {
    let e = Env::new();
    let r = e.run_safesh(&["log", "--date", "2000-01-01"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(
        r.stdout.contains("no log for 2000-01-01"),
        "{}{}",
        r.stdout,
        r.stderr
    );
}

/// Runs the hook for `cmd`, with `SAFESH_DEBUG` set when `debug` is given.
fn hook_stderr(e: &Env, cmd: &str, debug: Option<&str>) -> String {
    use std::io::Write as _;
    let mut c = e.safesh(&["hook", "claude"]);
    if let Some(v) = debug {
        c.env("SAFESH_DEBUG", v);
    }
    let mut child = c
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn hook");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(e.input(cmd, "s1").as_bytes())
        .unwrap();
    let o = child.wait_with_output().unwrap();
    assert_eq!(o.status.code(), Some(0));
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn debug_echoes_events_to_stderr() {
    let e = Env::new();
    let err = hook_stderr(&e, "echo hi", Some("1"));
    assert!(err.contains("[safesh] "), "{err}");
    assert!(err.contains("hook.decision"), "{err}");
    assert!(err.contains("hook.end"), "{err}");
}

#[test]
fn no_echo_without_debug() {
    let e = Env::new();
    assert!(!hook_stderr(&e, "echo hi", None).contains("[safesh]"));
    assert!(!hook_stderr(&e, "echo hi", Some("0")).contains("[safesh]"));
}

/// The script id in a hook rewrite: `... run --id <id> ...`.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn rewrite_id(command: &str) -> String {
    let i = command.find("--id ").expect("no --id") + 5;
    command[i..].split_whitespace().next().unwrap().to_string()
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn status_shows_child_and_output_file() {
    let e = Env::new();
    let h = e.hook("sleep 3");
    assert_eq!(h.decision(), "allow", "{}", h.stdout);
    let id = rewrite_id(&h.command());
    let out = e.root.join("out.txt");
    let mut run = e
        .safesh(&["run", "--id", &id])
        .stdout(std::fs::File::create(&out).unwrap())
        .stderr(std::fs::File::create(&out).unwrap())
        .spawn()
        .expect("spawn run");
    // Wait until the run has started its child.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let status = loop {
        let s = e.run_safesh(&["status"]).stdout;
        if s.contains("child=") || std::time::Instant::now() > deadline {
            break s;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    };
    let _ = run.kill();
    let _ = run.wait();
    assert!(status.contains("exec sleep 3"), "{status}");
    assert!(status.contains("child="), "{status}");
    let want = format!("output: {}", canon(&out).display());
    assert!(status.contains(&want), "{status}");
    assert!(!status.contains("stderr:"), "same file as stdout: {status}");
}

#[cfg(unix)]
#[test]
fn status_marks_an_exited_child() {
    let e = Env::new();
    let mut c = std::process::Command::new("true").spawn().unwrap();
    let dead = c.id();
    c.wait().unwrap();
    let dir = e.state.join("active");
    std::fs::create_dir_all(&dir).unwrap();
    let me = std::process::id();
    let v = serde_json::json!({
        "pid": me, "kind": "run", "id": "abc123", "step": "exec make",
        "started": "2026-01-01T00:00:00.000+00:00", "step_at": "2026-01-01T00:00:00.000+00:00",
        "child": dead,
    });
    std::fs::write(dir.join(format!("{me}.json")), v.to_string()).unwrap();
    let s = e.run_safesh(&["status"]).stdout;
    assert!(s.contains(&format!("child={dead} (exited)")), "{s}");
}
