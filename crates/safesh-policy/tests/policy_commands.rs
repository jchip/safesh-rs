mod policy_common;

use std::path::{Path, PathBuf};

use policy_common::*;
use safesh_policy::policy::{candidate_cwds, check_command_allowed_in_any_cwd};

fn cwds(command: &str, d: Option<&[Option<&str>]>) -> Vec<PathBuf> {
    let d: Option<Vec<Option<String>>> = d.map(|d| d.iter().map(|x| x.map(String::from)).collect());
    candidate_cwds(command, Path::new("/base"), d.as_deref())
}

fn paths(items: &[&str]) -> Vec<PathBuf> {
    items.iter().map(PathBuf::from).collect()
}

#[test]
fn cmd_27_bare_name_collapses() {
    assert_eq!(cwds("git", Some(&[Some("/ws")])), paths(&["/base"]));
}

#[test]
fn cmd_28_absolute_collapses() {
    assert_eq!(
        cwds("/usr/bin/tool", Some(&[Some("/ws")])),
        paths(&["/base"])
    );
}

#[test]
fn cmd_29_no_descriptors() {
    assert_eq!(cwds("./tool", None), paths(&["/base"]));
}

#[test]
#[cfg_attr(
    windows,
    ignore = "Unix path literals: on Windows /x is a Git Bash path"
)]
fn cmd_30_absolute_descriptor() {
    assert_eq!(
        cwds("./gradlew", Some(&[Some("/ws/pkg")])),
        paths(&["/base", "/ws/pkg"])
    );
}

#[test]
fn cmd_31_relative_descriptor() {
    assert_eq!(
        cwds("./tool", Some(&[Some("sub")])),
        paths(&["/base", "/base/sub"])
    );
}

#[test]
fn cmd_32_null_and_empty_fall_back() {
    assert_eq!(cwds("./tool", Some(&[None, Some("")])), paths(&["/base"]));
}

#[test]
fn cmd_33_cd_target_allows_gradlew() {
    let f = fx();
    let base = f.root.join("safesh");
    let ws = f.root.join("dev");
    let pkg = ws.join("fyntime/android");
    fs_err::create_dir_all(&base).unwrap();
    let gradlew = pkg.join("gradlew");
    exe(&gradlew);
    let mut c = cfg(&base);
    c.workspace_roots = Some(vec![ws.to_string_lossy().into()]);
    c.allow_project_commands = Some(true);
    let p = policy(&c, &base, &f.host);

    assert!(!p.check_run("./gradlew", &base).is_allow());

    let d = Some(vec![Some(pkg.to_string_lossy().into_owned())]);
    let list = candidate_cwds("./gradlew", &base, d.as_deref());
    assert_allow(
        &check_command_allowed_in_any_cwd(&p, "./gradlew", &list),
        &gradlew,
    );
}

#[test]
fn cmd_34_cd_outside_roots_not_allowed() {
    let f = fx();
    let base = f.root.join("safesh");
    let elsewhere = f.root.join("elsewhere");
    fs_err::create_dir_all(&base).unwrap();
    exe(&elsewhere.join("tool"));
    let mut c = cfg(&base);
    c.workspace_roots = Some(vec![f.root.join("dev").to_string_lossy().into()]);
    c.allow_project_commands = Some(true);
    let p = policy(&c, &base, &f.host);

    let d = Some(vec![Some(elsewhere.to_string_lossy().into_owned())]);
    let list = candidate_cwds("./tool", &base, d.as_deref());
    assert!(!check_command_allowed_in_any_cwd(&p, "./tool", &list).is_allow());
}
