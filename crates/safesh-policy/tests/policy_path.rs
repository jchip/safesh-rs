#![allow(
    clippy::default_trait_access,
    clippy::many_single_char_names,
    clippy::semicolon_if_nothing_returned,
    clippy::ref_option
)]
mod policy_common;

use std::path::Path;

use policy_common::*;
use safesh_policy::effect::Access;
use safesh_policy::policy::{Policy, PolicyOptions, load_session_config};

#[test]
fn pu_13_project_read_allowed() {
    let f = fx();
    let t = f.root.join("t");
    let p = policy(&cfg(&t), &t, &f.host);
    assert_allow(&read(&p, &t.join("file.txt"), &t), &t.join("file.txt"));
}

#[test]
fn pu_14_project_write_blocked() {
    let f = fx();
    let t = f.root.join("t");
    let mut c = cfg(&t);
    c.block_project_dir_write = Some(true);
    let p = policy(&c, &t, &f.host);
    assert_blocked(&write_access(&p, &t.join("file.txt"), &t));
}

#[test]
fn pu_15_project_write_allowed_when_not_blocked() {
    let f = fx();
    let t = f.root.join("t");
    let mut c = cfg(&t);
    c.block_project_dir_write = Some(false);
    let p = policy(&c, &t, &f.host);
    assert!(write_access(&p, &t.join("file.txt"), &t).is_allow());
}

#[test]
fn pu_16_explicit_read_allowed() {
    let f = fx();
    let t = f.root.join("t");
    let c = with_perms(Default::default(), |p| {
        p.read = Some(s(&[t.to_str().unwrap()]))
    });
    let p = policy(&c, &t, &f.host);
    assert!(read(&p, &t.join("file.txt"), &t).is_allow());
}

#[test]
#[ignore = "facade: cwd is always in default read, TS checkPathPermission has no defaults"]
fn pu_17_path_outside_read_list_not_allowed() {
    let f = fx();
    let t = f.root.join("t");
    let c = with_perms(Default::default(), |p| {
        p.read = Some(s(&["/some/other/path"]));
    });
    let p = policy(&c, &t, &f.host);
    assert_ask(&read(&p, &t.join("file.txt"), &t));
}

#[test]
fn pu_18_empty_config_not_allowed() {
    let f = fx();
    let t = f.root.join("t");
    let p = policy(&Default::default(), &t, &f.host);
    assert_ask(&read(&p, Path::new("/any/path"), &t));
}

#[test]
fn tp_12_symlink_to_outside_not_allowed() {
    let f = fx();
    let allowed = f.root.join("allowed");
    let secret = f.host.home.join(".safesh-test-symlink-outside/secret.txt");
    write(&secret, "x");
    fs_err::create_dir_all(&allowed).unwrap();
    std::os::unix::fs::symlink(&secret, allowed.join("link.txt")).unwrap();
    let mut c = with_perms(Default::default(), |p| {
        p.read = Some(s(&[allowed.to_str().unwrap()]));
    });
    c.include_home_in_default_read = Some(false);
    let cwd = f.root.join("cwd");
    let p = policy(&c, &cwd, &f.host);
    assert_ask(&read(&p, &allowed.join("link.txt"), &cwd));
}

#[test]
fn tp_13_symlink_resolves_to_target() {
    let f = fx();
    let dir = f.root.join("dir");
    write(&dir.join("target.txt"), "x");
    std::os::unix::fs::symlink(dir.join("target.txt"), dir.join("link.txt")).unwrap();
    let c = with_perms(Default::default(), |p| {
        p.read = Some(s(&[dir.to_str().unwrap()]))
    });
    let p = policy(&c, &f.root.join("cwd"), &f.host);
    let d = read(&p, &dir.join("link.txt"), &f.root);
    assert_allow(&d, &dir.join("target.txt"));
}

#[test]
fn tp_14_nonexistent_file_in_allowed_dir() {
    let f = fx();
    let dir = f.root.join("dir");
    fs_err::create_dir_all(&dir).unwrap();
    let c = with_perms(Default::default(), |p| {
        p.read = Some(s(&[dir.to_str().unwrap()]))
    });
    let p = policy(&c, &f.root.join("cwd"), &f.host);
    assert_allow(
        &read(&p, &dir.join("newfile.txt"), &f.root),
        &dir.join("newfile.txt"),
    );
}

#[test]
fn tp_15_etc_passwd_not_allowed() {
    let f = fx();
    let c = with_perms(Default::default(), |p| p.read = Some(s(&["/tmp/allowed"])));
    let p = policy(&c, &f.root.join("cwd"), &f.host);
    assert_ask(&read(&p, Path::new("/etc/passwd"), &f.root));
}

#[test]
fn tp_16_traversal_not_allowed() {
    let f = fx();
    let dir = f.root.join("traversal");
    fs_err::create_dir_all(&dir).unwrap();
    let c = with_perms(Default::default(), |p| {
        p.read = Some(s(&[dir.to_str().unwrap()]))
    });
    let p = policy(&c, &f.root.join("cwd"), &f.host);
    let escape = dir.join("../../../etc/passwd");
    assert_ask(&read(&p, &escape, &f.root));
}

#[test]
fn tp_17_read_and_write_lists_are_separate() {
    let f = fx();
    let c = with_perms(Default::default(), |p| {
        p.read = Some(s(&["/custom/readable"]));
        p.write = Some(s(&["/custom/writable"]));
    });
    let cwd = f.root.join("cwd");
    let p = policy(&c, &cwd, &f.host);
    assert_ask(&read(&p, Path::new("/custom/writable/file.txt"), &cwd));
    assert_ask(&write_access(
        &p,
        Path::new("/custom/readable/file.txt"),
        &cwd,
    ));
}

#[test]
fn pd_11_project_dir_implicit_read_write() {
    let f = fx();
    let t = f.root.join("project");
    let file = t.join("src/file.ts");
    write(&file, "x");
    let c = with_perms(cfg(&t), |p| {
        p.read = Some(vec![]);
        p.write = Some(vec![]);
    });
    let p = policy(&c, &f.root.join("cwd"), &f.host);
    assert_allow(&read(&p, &file, &f.root), &file);
    assert_allow(&write_access(&p, &file, &f.root), &file);
}

#[test]
fn pd_12_outside_project_not_allowed() {
    let f = fx();
    let t = f.root.join("project");
    fs_err::create_dir_all(&t).unwrap();
    let outside = f.host.home.join(".safesh-test-projdir-outside/file.ts");
    write(&outside, "x");
    let mut c = with_perms(cfg(&t), |p| {
        p.read = Some(vec![]);
        p.write = Some(vec![]);
    });
    c.include_home_in_default_read = Some(false);
    let p = policy(&c, &t, &f.host);
    assert_ask(&read(&p, &outside, &t));
}

fn block_fx(block: Option<bool>) -> (Fx, std::path::PathBuf, std::path::PathBuf, Policy) {
    let f = fx();
    let t = f.root.join("project");
    let file = t.join("src/file.ts");
    write(&file, "x");
    let mut c = with_perms(cfg(&t), |p| {
        p.read = Some(vec![]);
        p.write = Some(vec![]);
    });
    c.block_project_dir_write = block;
    let p = policy(&c, &f.root.join("cwd"), &f.host);
    (f, t, file, p)
}

#[test]
fn pd_25_block_write_true() {
    let (f, _t, file, p) = block_fx(Some(true));
    assert_allow(&read(&p, &file, &f.root), &file);
    assert_blocked(&write_access(&p, &file, &f.root));
}

#[test]
fn pd_26_block_write_false() {
    let (f, _t, file, p) = block_fx(Some(false));
    assert_allow(&read(&p, &file, &f.root), &file);
    assert_allow(&write_access(&p, &file, &f.root), &file);
}

#[test]
fn pd_27_block_write_unset() {
    let (f, _t, file, p) = block_fx(None);
    assert_allow(&read(&p, &file, &f.root), &file);
    assert_allow(&write_access(&p, &file, &f.root), &file);
}

fn explicit_write_fx(deny: Option<&str>) -> (Fx, std::path::PathBuf, Policy) {
    let f = fx();
    let t = f.root.join("project");
    fs_err::create_dir_all(t.join("build")).unwrap();
    let mut c = with_perms(cfg(&t), |p| {
        p.write = Some(s(&[t.join("build").to_str().unwrap()]));
        if let Some(d) = deny {
            p.deny_write = Some(s(&[&t.join(d).to_string_lossy()]));
        }
    });
    c.block_project_dir_write = Some(true);
    let p = policy(&c, &t, &f.host);
    (f, t, p)
}

#[test]
fn pd_37_explicit_write_punches_through_block() {
    let (_f, t, p) = explicit_write_fx(None);
    assert_allow(
        &write_access(&p, &t.join("build/out.txt"), &t),
        &t.join("build/out.txt"),
    );
    assert_blocked(&write_access(&p, &t.join("src.txt"), &t));
    assert_allow(&read(&p, &t.join("src.txt"), &t), &t.join("src.txt"));
}

#[test]
fn pd_38_user_deny_beats_explicit_write() {
    let (_f, t, p) = explicit_write_fx(Some("build/secrets"));
    assert_denied(&write_access(&p, &t.join("build/secrets/key.pem"), &t));
}

#[test]
fn pd_21_worktree_roots_allow_top_checkout() {
    let f = fx_outside_repo();
    let r = &f.root;
    let top = r.join("workflow-engine");
    let wt = r.join("full-local");
    fs_err::create_dir_all(top.join(".git/worktrees/full-local")).unwrap();
    let doc = top.join("notes/oauth-tenant-tech.md");
    write(&doc, "x");
    let cwd = wt.join("notes");
    fs_err::create_dir_all(&cwd).unwrap();
    write(
        &wt.join(".git"),
        &format!(
            "gitdir: {}\n",
            top.join(".git/worktrees/full-local").display()
        ),
    );

    let loaded = load_session_config(&cwd, None, &f.host).unwrap();
    assert_eq!(loaded.config.project_dir.as_deref(), wt.to_str());
    assert_eq!(
        loaded.config.workspace_roots,
        Some(s(&[wt.to_str().unwrap(), top.to_str().unwrap()]))
    );
    let p = Policy::new(&loaded.config, &cwd, &f.host, &PolicyOptions::default()).unwrap();
    assert_allow(&p.check_path(&doc, &cwd, Access::Read), &doc);
}
