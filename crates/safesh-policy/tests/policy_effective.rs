#![allow(
    clippy::default_trait_access,
    clippy::many_single_char_names,
    clippy::semicolon_if_nothing_returned,
    clippy::ref_option
)]
mod policy_common;

use std::ffi::OsString;
use std::path::Path;

use policy_common::*;
use safesh_policy::config::Config;
use safesh_policy::policy::effective_permissions;

fn has(list: &[String], item: &str) -> bool {
    list.iter().any(|x| x == item)
}

fn proj_cfg(block: Option<bool>) -> Config {
    Config {
        project_dir: Some("/project".into()),
        block_project_dir_write: block,
        permissions: Some(Default::default()),
        ..Config::default()
    }
}

#[test]
fn perm_5_tmpdir_in_default_read_write() {
    let f = fx();
    let mut host = f.host.clone();
    let tmp = f.root.join("os-tmp");
    fs_err::create_dir_all(&tmp).unwrap();
    host.tmpdir = Some(tmp.clone());
    let e = effective_permissions(&Config::default(), Path::new("/test/project"), &host);
    assert!(has(&e.read, tmp.to_str().unwrap()));
    assert!(has(&e.write, tmp.to_str().unwrap()));
}

#[test]
fn perm_6_tmpdir_snapshot_is_host_env() {
    let f = fx();
    let e = effective_permissions(&Config::default(), Path::new("/test/project"), &f.host);
    assert!(!has(&e.read, "/var/folders/0p/evil/T"));
    assert!(!has(&e.write, "/var/folders/0p/evil/T"));
    assert!(has(&e.write, "/tmp"));
}

#[test]
#[cfg_attr(
    windows,
    ignore = "Unix path literals: on Windows /x is a Git Bash path"
)]
fn perm_9_path_dirs_in_default_read() {
    let f = fx();
    let mut host = f.host.clone();
    host.path = Some(OsString::from("/opt/a/bin:/opt/b/bin/:relative:/opt/a/bin"));
    let e = effective_permissions(&Config::default(), Path::new("/test/project"), &host);
    assert!(has(&e.read, "/opt/a/bin"));
    assert!(has(&e.read, "/opt/b/bin"));
    assert!(!has(&e.read, "relative"));
}

#[test]
fn perm_10_path_dirs_can_be_excluded() {
    let f = fx();
    let mut host = f.host.clone();
    host.path = Some(OsString::from("/usr/bin:/bin"));
    let c = Config {
        project_dir: Some("/test/project".into()),
        include_path_dirs_in_default_read: Some(false),
        ..Config::default()
    };
    let e = effective_permissions(&c, Path::new("/test/project"), &host);
    assert!(!has(&e.read, "/usr/bin"));
    assert!(!has(&e.read, "/bin"));
}

#[test]
#[cfg_attr(
    windows,
    ignore = "Unix path literals: on Windows /x is a Git Bash path"
)]
fn tp_5_workspace_dir_in_read_and_write() {
    let f = fx();
    let c = Config {
        workspace_dir: Some("/workspace".into()),
        ..Config::default()
    };
    let e = effective_permissions(&c, Path::new("/project"), &f.host);
    assert!(has(&e.read, "/workspace"));
    assert!(has(&e.write, "/workspace"));
}

#[test]
#[cfg_attr(
    windows,
    ignore = "Unix path literals: on Windows /x is a Git Bash path"
)]
fn pd_30_block_write_denies_root() {
    let f = fx();
    let e = effective_permissions(&proj_cfg(Some(true)), Path::new("/tmp"), &f.host);
    assert!(has(&e.deny_write, "/project"));
    assert!(!has(&e.write, "/project"));
    assert!(has(&e.read, "/project"));
}

#[test]
#[cfg_attr(
    windows,
    ignore = "Unix path literals: on Windows /x is a Git Bash path"
)]
fn pd_31_block_write_false() {
    let f = fx();
    let e = effective_permissions(&proj_cfg(Some(false)), Path::new("/tmp"), &f.host);
    assert!(has(&e.write, "/project"));
    assert!(!has(&e.deny_write, "/project"));
    assert!(has(&e.read, "/project"));
}

#[test]
#[cfg_attr(
    windows,
    ignore = "Unix path literals: on Windows /x is a Git Bash path"
)]
fn pd_32_block_write_unset() {
    let f = fx();
    let e = effective_permissions(&proj_cfg(None), Path::new("/tmp"), &f.host);
    assert!(has(&e.write, "/project"));
    assert!(!has(&e.deny_write, "/project"));
}

#[test]
fn pd_33_home_read_not_write() {
    let f = fx();
    let c = Config {
        permissions: Some(Default::default()),
        ..Config::default()
    };
    let e = effective_permissions(&c, Path::new("/tmp"), &f.host);
    let home = f.host.home.to_str().unwrap();
    assert!(has(&e.read, home));
    assert!(!has(&e.write, home));
}

#[test]
fn pd_35_explicit_write_under_root_keeps_root_out_of_deny() {
    let f = fx();
    let mut c = proj_cfg(Some(true));
    c.permissions = Some(
        with_perms(Config::default(), |p| {
            p.write = Some(s(&["/project/build"]));
        })
        .permissions
        .unwrap(),
    );
    let e = effective_permissions(&c, Path::new("/project"), &f.host);
    assert!(!has(&e.deny_write, "/project"));
    assert!(has(&e.write, "/project/build"));
    assert!(!has(&e.write, "/project"));
}

#[test]
#[cfg_attr(
    windows,
    ignore = "Unix path literals: on Windows /x is a Git Bash path"
)]
fn pd_36_explicit_write_elsewhere_still_denies_root() {
    let f = fx();
    let mut c = proj_cfg(Some(true));
    c.permissions = Some(
        with_perms(Config::default(), |p| {
            p.write = Some(s(&["/elsewhere"]));
        })
        .permissions
        .unwrap(),
    );
    let e = effective_permissions(&c, Path::new("/project"), &f.host);
    assert!(has(&e.deny_write, "/project"));
}
