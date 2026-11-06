use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use safesh_hooks::{ApprovalRequest, consume_approval, write_approval};
use safesh_policy::host::HostEnv;

fn host(d: &tempfile::TempDir) -> HostEnv {
    HostEnv {
        home: d.path().join("home"),
        tmpdir: None,
        xdg_config_home: None,
        path: None,
        claude_project_dir: None,
        uid: 0,
        state_dir: d.path().join("state"),
    }
}

fn req(cmd: &str) -> ApprovalRequest {
    ApprovalRequest {
        session_id: "s".into(),
        turn_id: "t".into(),
        cwd: "/w".into(),
        command: cmd.into(),
    }
}

fn files(h: &HostEnv) -> Vec<PathBuf> {
    std::fs::read_dir(h.state_dir.join("approvals"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect()
}

#[test]
fn write_then_consume_once_with_modes() {
    let d = tempfile::tempdir().unwrap();
    let h = host(&d);
    write_approval(&h, &req("a"), 1_000).unwrap();
    let dir = h.state_dir.join("approvals");
    assert_eq!(
        std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let f = files(&h);
    assert_eq!(f.len(), 1);
    assert_eq!(
        std::fs::metadata(&f[0]).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let body: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&f[0]).unwrap()).unwrap();
    assert_eq!(body["sessionId"], "s");
    assert_eq!(body["createdAt"], 1_000);
    assert!(consume_approval(&h, &req("a"), 2_000).unwrap());
    assert!(!consume_approval(&h, &req("a"), 2_000).unwrap());
    assert!(files(&h).is_empty());
}

#[test]
fn mismatch_is_kept() {
    let d = tempfile::tempdir().unwrap();
    let h = host(&d);
    write_approval(&h, &req("a"), 1_000).unwrap();
    assert!(!consume_approval(&h, &req("b"), 1_000).unwrap());
    assert_eq!(files(&h).len(), 1);
}

#[test]
fn expired_is_removed() {
    let d = tempfile::tempdir().unwrap();
    let h = host(&d);
    write_approval(&h, &req("a"), 1_000).unwrap();
    assert!(!consume_approval(&h, &req("a"), 1_000 + 60_001).unwrap());
    assert!(files(&h).is_empty());
    write_approval(&h, &req("a"), 1_000).unwrap();
    assert!(consume_approval(&h, &req("a"), 1_000 + 60_000).unwrap());
}

#[test]
fn write_cleans_expired() {
    let d = tempfile::tempdir().unwrap();
    let h = host(&d);
    write_approval(&h, &req("old"), 1_000).unwrap();
    write_approval(&h, &req("new"), 100_000).unwrap();
    assert_eq!(files(&h).len(), 1);
}
