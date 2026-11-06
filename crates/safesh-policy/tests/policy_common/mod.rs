#![allow(dead_code)]

use std::path::{Path, PathBuf};

use safesh_policy::config::{Config, PermissionsConfig};
use safesh_policy::effect::{Access, Decision, Reason};
use safesh_policy::host::HostEnv;
use safesh_policy::policy::{Policy, PolicyOptions};
use tempfile::TempDir;

/// Temp dir under the cargo target tmp dir, so it never sits under `/tmp` defaults.
pub struct Fx {
    pub _t: TempDir,
    /// Canonical root of the temp dir.
    pub root: PathBuf,
    pub host: HostEnv,
}

pub fn fx() -> Fx {
    make_fx(
        tempfile::Builder::new()
            .prefix("policy-")
            .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
            .unwrap(),
    )
}

/// Like [`fx`] but under the OS temp dir, away from any `.claude` dir above the repo.
/// Needed by project root detection tests.
pub fn fx_outside_repo() -> Fx {
    make_fx(
        tempfile::Builder::new()
            .prefix("policy-")
            .tempdir()
            .unwrap(),
    )
}

fn make_fx(t: TempDir) -> Fx {
    let root = fs_err::canonicalize(t.path()).unwrap();
    fs_err::create_dir_all(root.join("emptybin")).unwrap();
    let host = HostEnv {
        home: root.join("home"),
        tmpdir: None,
        xdg_config_home: None,
        path: Some(root.join("emptybin").into_os_string()),
        claude_project_dir: None,
        uid: 0,
        state_dir: root.join("state"),
    };
    Fx { _t: t, root, host }
}

pub fn write(path: &Path, text: &str) {
    fs_err::create_dir_all(path.parent().unwrap()).unwrap();
    fs_err::write(path, text).unwrap();
}

/// Creates an executable file.
pub fn exe(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    write(path, "#!/bin/sh\n");
    fs_err::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

pub fn s(items: &[&str]) -> Vec<String> {
    items.iter().map(|x| (*x).to_string()).collect()
}

pub fn cfg(project_dir: &Path) -> Config {
    Config {
        project_dir: Some(project_dir.to_string_lossy().into_owned()),
        ..Config::default()
    }
}

pub fn with_run(mut c: Config, run: &[&str]) -> Config {
    c.permissions
        .get_or_insert_with(PermissionsConfig::default)
        .run = Some(s(run));
    c
}

pub fn with_perms(mut c: Config, f: impl FnOnce(&mut PermissionsConfig)) -> Config {
    f(c.permissions.get_or_insert_with(PermissionsConfig::default));
    c
}

pub fn policy(c: &Config, cwd: &Path, host: &HostEnv) -> Policy {
    Policy::new(c, cwd, host, &PolicyOptions::default()).unwrap()
}

pub fn policy_session(c: &Config, cwd: &Path, host: &HostEnv) -> Policy {
    let opts = PolicyOptions {
        session_id: Some("sid".into()),
    };
    Policy::new(c, cwd, host, &opts).unwrap()
}

pub fn assert_allow(d: &Decision, resolved: &Path) {
    assert_eq!(
        d,
        &Decision::Allow {
            resolved: resolved.into()
        }
    );
}

pub fn assert_ask(d: &Decision) {
    assert!(
        matches!(
            d,
            Decision::Ask {
                reason: Reason::NotAllowed,
                ..
            }
        ),
        "{d:?}"
    );
}

pub fn assert_not_found(d: &Decision) {
    assert!(
        matches!(
            d,
            Decision::Deny {
                reason: Reason::NotFound,
                ..
            }
        ),
        "{d:?}"
    );
}

pub fn assert_denied(d: &Decision) {
    assert!(
        matches!(
            d,
            Decision::Deny {
                reason: Reason::Denied,
                ..
            }
        ),
        "{d:?}"
    );
}

pub fn assert_blocked(d: &Decision) {
    assert!(
        matches!(
            d,
            Decision::Deny {
                reason: Reason::ProjectDirWriteBlocked,
                ..
            }
        ),
        "{d:?}"
    );
}

pub fn read(p: &Policy, path: &Path, cwd: &Path) -> Decision {
    p.check_path(path, cwd, Access::Read)
}

pub fn write_access(p: &Policy, path: &Path, cwd: &Path) -> Decision {
    p.check_path(path, cwd, Access::Write)
}
