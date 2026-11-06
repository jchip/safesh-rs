#![allow(
    clippy::default_trait_access,
    clippy::many_single_char_names,
    clippy::semicolon_if_nothing_returned,
    clippy::ref_option
)]
mod policy_common;

use policy_common::*;
use safesh_policy::config::{Config, PermissionsConfig};
use safesh_policy::policy::merge_session_permissions;
use safesh_policy::state::session::{add_session_commands, add_session_paths};
use safesh_policy::state::temp::session_file_path;

const SID: &str = "sid";

fn config(read: Option<&[&str]>, write: Option<&[&str]>, run: Option<&[&str]>) -> Config {
    let conv = |x: Option<&[&str]>| x.map(s);
    Config {
        permissions: Some(PermissionsConfig {
            read: conv(read),
            write: conv(write),
            run: conv(run),
            ..PermissionsConfig::default()
        }),
        ..Config::default()
    }
}

fn merged(f: &Fx, mut c: Config) -> Config {
    merge_session_permissions(&mut c, &f.root.join("project"), SID, &f.host);
    c
}

fn seed(f: &Fx, cmds: &[&str], read: &[&str], write: &[&str]) {
    let dir = f.root.join("project");
    if !cmds.is_empty() {
        add_session_commands(&f.host, &s(cmds), Some(&dir), SID).unwrap();
    }
    if !read.is_empty() || !write.is_empty() {
        add_session_paths(&f.host, &s(read), &s(write), Some(&dir), SID).unwrap();
    }
}

fn contains_all(list: &Option<Vec<String>>, items: &[&str]) {
    let list = list.as_ref().unwrap();
    for i in items {
        assert!(list.iter().any(|x| x == i), "{i} missing from {list:?}");
    }
}

#[test]
fn ses_25_read_merged() {
    let f = fx();
    seed(&f, &[], &["/etc/hosts", "/etc/passwd"], &[]);
    let c = merged(&f, config(Some(&["/var/log"]), None, None));
    let read = c.permissions.unwrap().read;
    assert_eq!(read.as_ref().unwrap().len(), 3);
    contains_all(&read, &["/var/log", "/etc/hosts", "/etc/passwd"]);
}

#[test]
fn ses_26_write_merged() {
    let f = fx();
    seed(&f, &[], &[], &["/tmp/output.txt"]);
    let c = merged(&f, config(None, Some(&["/var/app.log"]), None));
    let write = c.permissions.unwrap().write;
    assert_eq!(write.as_ref().unwrap().len(), 2);
    contains_all(&write, &["/var/app.log", "/tmp/output.txt"]);
}

#[test]
fn ses_27_commands_merged() {
    let f = fx();
    seed(&f, &["docker", "kubectl"], &[], &[]);
    let c = merged(&f, config(None, None, Some(&["git"])));
    let run = c.permissions.unwrap().run;
    assert_eq!(run.as_ref().unwrap().len(), 3);
    contains_all(&run, &["git", "docker", "kubectl"]);
}

#[test]
fn ses_28_permissions_created() {
    let f = fx();
    seed(&f, &["docker"], &["/etc/hosts"], &["/tmp/out"]);
    let c = merged(&f, Config::default());
    let p = c.permissions.unwrap();
    assert_eq!(p.read, Some(s(&["/etc/hosts"])));
    assert_eq!(p.write, Some(s(&["/tmp/out"])));
    assert_eq!(p.run, Some(s(&["docker"])));
}

#[test]
fn ses_29_no_session_file_unchanged() {
    let f = fx();
    let c = merged(&f, config(Some(&["/var/log"]), None, None));
    assert_eq!(c.permissions.unwrap().read, Some(s(&["/var/log"])));
}

#[test]
fn ses_30_all_merged() {
    let f = fx();
    seed(
        &f,
        &["npm", "yarn"],
        &["/etc/hosts", "/etc/passwd"],
        &["/tmp/log.txt", "/var/app.log"],
    );
    let c = merged(
        &f,
        config(
            Some(&["/home/user/.bashrc"]),
            Some(&["/tmp/test.txt"]),
            Some(&["git"]),
        ),
    );
    let p = c.permissions.unwrap();
    assert_eq!(p.read.as_ref().unwrap().len(), 3);
    assert_eq!(p.write.as_ref().unwrap().len(), 3);
    assert_eq!(p.run.as_ref().unwrap().len(), 3);
    contains_all(&p.run, &["npm", "yarn", "git"]);
    contains_all(&p.write, &["/var/app.log"]);
}

#[test]
fn ses_31_corrupt_session_file_unchanged() {
    let f = fx();
    let dir = f.root.join("project");
    let path = session_file_path(&f.host, Some(&dir), SID).unwrap();
    write(&path, "not json at all");
    let c = merged(&f, config(Some(&["/var/log"]), None, None));
    assert_eq!(c.permissions.unwrap().read, Some(s(&["/var/log"])));
}
