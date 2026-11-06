use std::path::{Path, PathBuf};

use safesh_policy::host::HostEnv;
use safesh_policy::state::session::*;
use safesh_policy::state::temp::session_file_path;
use tempfile::TempDir;

const SID: &str = "test-session";

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

struct Fx {
    _t: TempDir,
    env: HostEnv,
    proj: PathBuf,
}

fn fx() -> Fx {
    let t = TempDir::new().unwrap();
    let env = env(t.path());
    let proj = t.path().join("proj");
    Fx { _t: t, env, proj }
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| (*x).to_string()).collect()
}

fn write_raw(f: &Fx, content: &str) {
    let p = session_file_path(&f.env, Some(&f.proj), SID).unwrap();
    std::fs::write(p, content).unwrap();
}

#[test]
fn ses_1_read_allowed_commands() {
    let f = fx();
    write_raw(&f, r#"{"allowedCommands":["docker","kubectl"]}"#);
    let d = read_session_file(&f.env, Some(&f.proj), SID);
    assert_eq!(d.allowed_commands, Some(s(&["docker", "kubectl"])));
}

#[test]
fn ses_2_read_permissions() {
    let f = fx();
    write_raw(
        &f,
        r#"{"permissions":{"read":["/etc/hosts"],"write":["/var/log/app.log"]}}"#,
    );
    let p = read_session_file(&f.env, Some(&f.proj), SID)
        .permissions
        .unwrap();
    assert_eq!(p.read, Some(s(&["/etc/hosts"])));
    assert_eq!(p.write, Some(s(&["/var/log/app.log"])));
}

#[test]
fn ses_3_read_missing() {
    let f = fx();
    assert_eq!(
        read_session_file(&f.env, Some(&f.proj), SID),
        SessionData::default()
    );
}

#[test]
fn ses_4_path_with_project() {
    let f = fx();
    let p = session_file_path(&f.env, Some(&f.proj), SID).unwrap();
    assert!(p.starts_with(&f.proj));
    assert!(p.to_string_lossy().contains("/.temp/safesh/"));
    assert!(p.ends_with("session-test-session.json"));
}

#[test]
fn ses_5_path_without_project() {
    let f = fx();
    let p = session_file_path(&f.env, None, SID).unwrap();
    assert!(p.starts_with(&f.env.state_dir));
    assert!(!p.starts_with(&f.proj));
}

#[test]
fn ses_6_write_new() {
    let f = fx();
    let d = SessionData {
        allowed_commands: Some(s(&["git", "npm"])),
        ..Default::default()
    };
    write_session_file(&f.env, d, Some(&f.proj), SID).unwrap();
    let p = session_file_path(&f.env, Some(&f.proj), SID).unwrap();
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
    assert_eq!(v["allowedCommands"], serde_json::json!(["git", "npm"]));
}

#[test]
fn ses_7_write_shallow_merge() {
    let f = fx();
    let a = SessionData {
        allowed_commands: Some(s(&["docker"])),
        ..Default::default()
    };
    write_session_file(&f.env, a, Some(&f.proj), SID).unwrap();
    let b = SessionData {
        permissions: Some(SessionPermissions {
            read: Some(s(&["/etc/passwd"])),
            write: None,
        }),
        ..Default::default()
    };
    write_session_file(&f.env, b, Some(&f.proj), SID).unwrap();
    let d = read_session_file(&f.env, Some(&f.proj), SID);
    assert_eq!(d.allowed_commands, Some(s(&["docker"])));
    assert_eq!(d.permissions.unwrap().read, Some(s(&["/etc/passwd"])));
}

#[test]
fn ses_8_write_overwrites_field() {
    let f = fx();
    for cmds in [&["old", "command"][..], &["new", "commands"][..]] {
        let d = SessionData {
            allowed_commands: Some(s(cmds)),
            ..Default::default()
        };
        write_session_file(&f.env, d, Some(&f.proj), SID).unwrap();
    }
    let d = read_session_file(&f.env, Some(&f.proj), SID);
    assert_eq!(d.allowed_commands, Some(s(&["new", "commands"])));
}

#[test]
fn ses_9_add_commands_new() {
    let f = fx();
    add_session_commands(&f.env, &s(&["docker", "kubectl"]), Some(&f.proj), SID).unwrap();
    let v = get_session_allowed_commands_vec(&f.env, Some(&f.proj), SID);
    assert_eq!(v, s(&["docker", "kubectl"]));
}

#[test]
fn ses_10_add_commands_accumulate() {
    let f = fx();
    add_session_commands(&f.env, &s(&["docker"]), Some(&f.proj), SID).unwrap();
    add_session_commands(&f.env, &s(&["kubectl"]), Some(&f.proj), SID).unwrap();
    assert_eq!(
        get_session_allowed_commands_vec(&f.env, Some(&f.proj), SID).len(),
        2
    );
}

#[test]
fn ses_11_add_commands_dedup() {
    let f = fx();
    add_session_commands(&f.env, &s(&["docker", "git"]), Some(&f.proj), SID).unwrap();
    add_session_commands(&f.env, &s(&["docker", "npm"]), Some(&f.proj), SID).unwrap();
    let v = get_session_allowed_commands_vec(&f.env, Some(&f.proj), SID);
    assert_eq!(v, s(&["docker", "git", "npm"]));
}

#[test]
fn ses_12_add_paths_new() {
    let f = fx();
    add_session_paths(
        &f.env,
        &s(&["/etc/hosts"]),
        &s(&["/var/log/app.log"]),
        Some(&f.proj),
        SID,
    )
    .unwrap();
    let p = get_session_path_permissions(&f.env, Some(&f.proj), SID);
    assert_eq!(p.read, Some(s(&["/etc/hosts"])));
    assert_eq!(p.write, Some(s(&["/var/log/app.log"])));
}

#[test]
fn ses_13_add_paths_accumulate() {
    let f = fx();
    add_session_paths(&f.env, &s(&["/etc/hosts"]), &[], Some(&f.proj), SID).unwrap();
    add_session_paths(&f.env, &s(&["/etc/passwd"]), &[], Some(&f.proj), SID).unwrap();
    let p = get_session_path_permissions(&f.env, Some(&f.proj), SID);
    assert_eq!(p.read, Some(s(&["/etc/hosts", "/etc/passwd"])));
}

#[test]
fn ses_14_add_paths_dedup() {
    let f = fx();
    add_session_paths(
        &f.env,
        &s(&["/etc/hosts", "/etc/passwd"]),
        &[],
        Some(&f.proj),
        SID,
    )
    .unwrap();
    add_session_paths(
        &f.env,
        &s(&["/etc/hosts", "/var/log"]),
        &[],
        Some(&f.proj),
        SID,
    )
    .unwrap();
    let p = get_session_path_permissions(&f.env, Some(&f.proj), SID);
    assert_eq!(p.read.unwrap().len(), 3);
}

#[test]
fn ses_15_add_paths_independent_lists() {
    let f = fx();
    add_session_paths(
        &f.env,
        &s(&["/etc/hosts"]),
        &s(&["/etc/hosts"]),
        Some(&f.proj),
        SID,
    )
    .unwrap();
    let p = get_session_path_permissions(&f.env, Some(&f.proj), SID);
    assert_eq!(p.read, Some(s(&["/etc/hosts"])));
    assert_eq!(p.write, Some(s(&["/etc/hosts"])));
}

#[test]
fn ses_16_add_paths_empty_creates_permissions() {
    let f = fx();
    add_session_paths(&f.env, &[], &[], Some(&f.proj), SID).unwrap();
    assert!(
        read_session_file(&f.env, Some(&f.proj), SID)
            .permissions
            .is_some()
    );
}

#[test]
fn ses_17_allowed_commands_set() {
    let f = fx();
    add_session_commands(&f.env, &s(&["docker", "git"]), Some(&f.proj), SID).unwrap();
    let set = get_session_allowed_commands(&f.env, Some(&f.proj), SID);
    assert_eq!(set.len(), 2);
    assert!(set.contains("docker") && set.contains("git"));
}

#[test]
fn ses_18_allowed_commands_empty() {
    let f = fx();
    assert!(get_session_allowed_commands(&f.env, Some(&f.proj), SID).is_empty());
}

#[test]
fn ses_19_allowed_commands_vec() {
    let f = fx();
    add_session_commands(&f.env, &s(&["npm", "yarn"]), Some(&f.proj), SID).unwrap();
    let v = get_session_allowed_commands_vec(&f.env, Some(&f.proj), SID);
    assert!(v.len() == 2 && v.contains(&"npm".to_string()) && v.contains(&"yarn".to_string()));
}

#[test]
fn ses_20_allowed_commands_vec_empty() {
    let f = fx();
    assert!(get_session_allowed_commands_vec(&f.env, Some(&f.proj), SID).is_empty());
}

#[test]
fn ses_21_path_permissions() {
    let f = fx();
    add_session_paths(
        &f.env,
        &s(&["/etc/hosts"]),
        &s(&["/var/log"]),
        Some(&f.proj),
        SID,
    )
    .unwrap();
    let p = get_session_path_permissions(&f.env, Some(&f.proj), SID);
    assert_eq!(p.read, Some(s(&["/etc/hosts"])));
    assert_eq!(p.write, Some(s(&["/var/log"])));
}

#[test]
fn ses_22_path_permissions_empty() {
    let f = fx();
    let p = get_session_path_permissions(&f.env, Some(&f.proj), SID);
    assert_eq!((p.read, p.write), (None, None));
}

#[test]
fn ses_23_project_dir_flow() {
    let f = fx();
    add_session_commands(&f.env, &s(&["docker"]), Some(&f.proj), SID).unwrap();
    let proj_file = session_file_path(&f.env, Some(&f.proj), SID).unwrap();
    let tmp_file = session_file_path(&f.env, None, SID).unwrap();
    assert!(proj_file.exists());
    assert!(!tmp_file.exists());
    assert_ne!(proj_file, tmp_file);
    assert!(get_session_allowed_commands(&f.env, Some(&f.proj), SID).contains("docker"));
    assert!(!get_session_allowed_commands(&f.env, None, SID).contains("docker"));
}

#[test]
fn ses_24_paths_differ() {
    let f = fx();
    let a = session_file_path(&f.env, Some(&f.proj), SID).unwrap();
    let b = session_file_path(&f.env, None, SID).unwrap();
    assert_ne!(a, b);
    assert!(a.starts_with(&f.proj));
    assert!(!b.starts_with(&f.proj));
    assert!(b.starts_with(&f.env.state_dir));
}

#[test]
fn ses_extra_corrupt_file_reads_empty() {
    let f = fx();
    write_raw(&f, "not json at all");
    assert_eq!(
        read_session_file(&f.env, Some(&f.proj), SID),
        SessionData::default()
    );
}

#[test]
fn ses_extra_unknown_keys_preserved() {
    let f = fx();
    write_raw(&f, r#"{"other":1}"#);
    add_session_commands(&f.env, &s(&["git"]), Some(&f.proj), SID).unwrap();
    let p = session_file_path(&f.env, Some(&f.proj), SID).unwrap();
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
    assert_eq!(v["other"], 1);
}

#[test]
fn session_and_pending_ids_cannot_escape_state_dir() {
    use safesh_policy::state::temp::{pending_file_path, session_file_path};
    let t = tempfile::TempDir::new().unwrap();
    let env = env(t.path());
    for bad in ["../x", "a/b", "", "a.b"] {
        assert!(session_file_path(&env, None, bad).is_err(), "{bad}");
        assert!(pending_file_path(&env, bad).is_err(), "{bad}");
    }
    assert!(session_file_path(&env, None, "abc-123_X").is_ok());
    assert!(pending_file_path(&env, "1730000000000-4242").is_ok());
}
