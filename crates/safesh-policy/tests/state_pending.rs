#![allow(clippy::many_single_char_names)]
use std::path::Path;

use safesh_policy::host::HostEnv;
use safesh_policy::state::pending::*;
use safesh_policy::state::temp::{pending_file_path, pending_path_file_path};
use tempfile::TempDir;

fn env(root: &Path) -> HostEnv {
    HostEnv {
        home: root.join("home"),
        tmpdir: None,
        xdg_config_home: None,
        path: None,
        claude_project_dir: None,
        uid: 0,
        state_dir: root.join("state"),
    }
}

fn cmd(id: &str) -> PendingCommand {
    PendingCommand {
        id: id.into(),
        script_hash: "abc123".into(),
        commands: vec!["docker".into(), "kubectl".into()],
        cwd: "/test/cwd".into(),
        project_dir: None,
        timeout: Some(5000),
        run_in_background: Some(false),
        created_at: "2026-01-01T00:00:00.000Z".into(),
    }
}

fn path_req(id: &str) -> PendingPathRequest {
    PendingPathRequest {
        id: id.into(),
        path: "/etc/hosts".into(),
        operation: PathOperation::Read,
        cwd: "/test/cwd".into(),
        script_hash: "def456".into(),
        run_in_background: None,
        created_at: "2026-01-01T00:00:00.000Z".into(),
    }
}

#[test]
fn pend_1_ids_differ() {
    let a = generate_pending_id();
    std::thread::sleep(std::time::Duration::from_millis(2));
    let b = generate_pending_id();
    assert_ne!(a, b);
    assert!(!a.is_empty() && !b.is_empty());
}

#[test]
fn pend_2_id_format() {
    let id = generate_pending_id();
    let parts: Vec<&str> = id.split('-').collect();
    assert_eq!(parts.len(), 2);
    assert!(parts[0].parse::<u64>().unwrap() > 0);
    assert!(parts[1].parse::<u32>().unwrap() > 0);
}

#[test]
fn pend_3_write_command() {
    let t = TempDir::new().unwrap();
    let e = env(t.path());
    write_pending_command(&e, &cmd("1-2")).unwrap();
    let p = pending_file_path(&e, "1-2").unwrap();
    assert!(p.starts_with(e.state_dir.join("pending")));
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
    assert_eq!(
        v,
        serde_json::json!({"id":"1-2","scriptHash":"abc123","commands":["docker","kubectl"],
            "cwd":"/test/cwd","timeout":5000,"runInBackground":false,
            "createdAt":"2026-01-01T00:00:00.000Z"})
    );
}

#[test]
fn pend_4_optional_fields_omitted() {
    let t = TempDir::new().unwrap();
    let e = env(t.path());
    let mut c = cmd("1-2");
    c.timeout = None;
    c.run_in_background = None;
    write_pending_command(&e, &c).unwrap();
    let v: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(pending_file_path(&e, "1-2").unwrap()).unwrap(),
    )
    .unwrap();
    let o = v.as_object().unwrap();
    assert!(!o.contains_key("timeout") && !o.contains_key("runInBackground"));
    assert!(!o.contains_key("projectDir"));
}

#[test]
fn pend_5_write_path() {
    let t = TempDir::new().unwrap();
    let e = env(t.path());
    write_pending_path(&e, &path_req("3-4")).unwrap();
    let p = pending_path_file_path(&e, "3-4").unwrap();
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
    assert_eq!(
        v,
        serde_json::json!({"id":"3-4","path":"/etc/hosts","operation":"read","cwd":"/test/cwd",
            "scriptHash":"def456","createdAt":"2026-01-01T00:00:00.000Z"})
    );
}

#[test]
fn pend_6_write_path_operation() {
    let t = TempDir::new().unwrap();
    let e = env(t.path());
    let mut r = path_req("5-6");
    r.operation = PathOperation::Write;
    write_pending_path(&e, &r).unwrap();
    let s = std::fs::read_to_string(pending_path_file_path(&e, "5-6").unwrap()).unwrap();
    let v: serde_json::Value = serde_json::from_str(&s).unwrap();
    assert_eq!(v["operation"], "write");
}

#[test]
fn pend_7_read_command_roundtrip() {
    let t = TempDir::new().unwrap();
    let e = env(t.path());
    let mut c = cmd("7-8");
    c.project_dir = Some("/proj".into());
    write_pending_command(&e, &c).unwrap();
    assert_eq!(read_pending_command(&e, "7-8"), Some(c));
}

#[test]
fn pend_8_read_command_missing() {
    let t = TempDir::new().unwrap();
    assert_eq!(
        read_pending_command(&env(t.path()), "nonexistent-12345"),
        None
    );
}

#[test]
fn pend_9_read_command_corrupt() {
    let t = TempDir::new().unwrap();
    let e = env(t.path());
    std::fs::write(pending_file_path(&e, "9-9").unwrap(), "{ invalid json }").unwrap();
    assert_eq!(read_pending_command(&e, "9-9"), None);
}

#[test]
fn pend_10_read_path_roundtrip() {
    let t = TempDir::new().unwrap();
    let e = env(t.path());
    let r = path_req("10-1");
    write_pending_path(&e, &r).unwrap();
    assert_eq!(read_pending_path(&e, "10-1"), Some(r));
}

#[test]
fn pend_11_read_path_missing() {
    let t = TempDir::new().unwrap();
    assert_eq!(read_pending_path(&env(t.path()), "nonexistent-67890"), None);
}

#[test]
fn pend_12_read_path_corrupt() {
    let t = TempDir::new().unwrap();
    let e = env(t.path());
    std::fs::write(
        pending_path_file_path(&e, "12-1").unwrap(),
        "not valid json at all",
    )
    .unwrap();
    assert_eq!(read_pending_path(&e, "12-1"), None);
}

#[test]
fn pend_13_delete_command() {
    let t = TempDir::new().unwrap();
    let e = env(t.path());
    write_pending_command(&e, &cmd("13-1")).unwrap();
    let p = pending_file_path(&e, "13-1").unwrap();
    assert!(p.exists());
    delete_pending(&e, "13-1", PendingKind::Command);
    assert!(!p.exists());
}

#[test]
fn pend_14_delete_path() {
    let t = TempDir::new().unwrap();
    let e = env(t.path());
    write_pending_path(&e, &path_req("14-1")).unwrap();
    let p = pending_path_file_path(&e, "14-1").unwrap();
    delete_pending(&e, "14-1", PendingKind::Path);
    assert!(!p.exists());
}

#[test]
fn pend_15_delete_missing_command() {
    let t = TempDir::new().unwrap();
    delete_pending(
        &env(t.path()),
        "missing-command-12345",
        PendingKind::Command,
    );
}

#[test]
fn pend_16_delete_missing_path() {
    let t = TempDir::new().unwrap();
    delete_pending(&env(t.path()), "missing-path-67890", PendingKind::Path);
}

#[test]
fn pend_17_delete_idempotent() {
    let t = TempDir::new().unwrap();
    let e = env(t.path());
    write_pending_command(&e, &cmd("17-1")).unwrap();
    for _ in 0..3 {
        delete_pending(&e, "17-1", PendingKind::Command);
    }
}
