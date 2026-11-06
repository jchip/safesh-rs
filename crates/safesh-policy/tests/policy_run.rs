#![allow(
    clippy::default_trait_access,
    clippy::many_single_char_names,
    clippy::semicolon_if_nothing_returned,
    clippy::ref_option
)]
mod policy_common;

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use policy_common::*;
use safesh_policy::config::Config;
use safesh_policy::effect::{Decision, Reason};
use safesh_policy::policy::{Policy, PolicyOptions, merge_session_permissions};
use safesh_policy::state::session::add_session_commands;

/// Policy with `run` from config plus `session` commands merged from a real session file.
fn session_policy(run: &[&str], session: &[&str]) -> (Fx, Policy) {
    let f = fx();
    let dir = f.root.join("project");
    if !session.is_empty() {
        add_session_commands(&f.host, &s(session), Some(&dir), "sid").unwrap();
    }
    let mut c = with_run(cfg(&dir), run);
    merge_session_permissions(&mut c, &dir, "sid", &f.host);
    let p = policy(&c, &home(), &f.host);
    (f, p)
}

fn home() -> PathBuf {
    PathBuf::from("/home/user")
}

fn run_policy(run: &[&str]) -> (Fx, Policy) {
    let f = fx();
    let c = with_run(cfg(Path::new("/home/user/project")), run);
    let p = policy(&c, &home(), &f.host);
    (f, p)
}

#[test]
fn cmd_1_run_set() {
    let (_f, p) = run_policy(&["git", "deno"]);
    assert_allow(&p.check_run("git", &home()), Path::new("git"));
    assert_allow(&p.check_run("deno", &home()), Path::new("deno"));
    assert_ask(&p.check_run("curl", &home()));
    assert_ask(&p.check_run("docker", &home()));
}

#[test]
fn cmd_2_membership() {
    let (_f, p) = run_policy(&["git", "deno"]);
    assert!(p.check_run("git", &home()).is_allow());
    assert!(p.check_run("deno", &home()).is_allow());
    assert!(!p.check_run("curl", &home()).is_allow());
}

#[test]
fn cmd_3_name_allowed() {
    let (_f, p) = run_policy(&["git"]);
    assert_allow(&p.check_run("git", &home()), Path::new("git"));
}

#[test]
fn cmd_4_name_not_allowed() {
    let (_f, p) = run_policy(&["git"]);
    assert_eq!(
        p.check_run("curl", &home()),
        Decision::Ask {
            resolved: "curl".into(),
            reason: Reason::NotAllowed
        }
    );
}

/// Host whose PATH holds `bin/<name>` and a second `other/<name>`.
fn path_host(f: &Fx, name: &str) -> (PathBuf, PathBuf, safesh_policy::host::HostEnv) {
    let bin = f.root.join("bin");
    let other = f.root.join("other");
    exe(&bin.join(name));
    exe(&other.join(name));
    let mut host = f.host.clone();
    host.path = Some(OsString::from(bin.as_os_str()));
    (bin.join(name), other.join(name), host)
}

#[test]
fn cmd_5_path_resolved_program_allowed() {
    let f = fx();
    let (good, bad, host) = path_host(&f, "git");
    let c = with_run(cfg(&f.root.join("project")), &["git"]);
    let p = policy(&c, &f.root, &host);
    assert_allow(&p.check_run(good.to_str().unwrap(), &f.root), &good);
    assert_ask(&p.check_run(bad.to_str().unwrap(), &f.root));
}

#[test]
fn cmd_6_verbatim_path_in_run_set() {
    let (_f, p) = run_policy(&["/usr/local/bin/custom-tool"]);
    assert_allow(
        &p.check_run("/usr/local/bin/custom-tool", &home()),
        Path::new("/usr/local/bin/custom-tool"),
    );
}

#[test]
fn cmd_7_other_absolute_not_allowed() {
    let (_f, p) = run_policy(&["git"]);
    assert_ask(&p.check_run("/usr/bin/curl", &home()));
}

#[test]
fn cmd_8_missing_relative_not_found() {
    let (_f, p) = run_policy(&[]);
    assert_not_found(&p.check_run("./nonexistent.sh", &home()));
}

struct Proj {
    f: Fx,
    dir: PathBuf,
    script: PathBuf,
}

fn proj() -> Proj {
    let f = fx();
    let dir = f.root.join("project");
    let script = dir.join("scripts/build.sh");
    exe(&script);
    Proj { f, dir, script }
}

fn proj_cfg(pr: &Proj, allow: Option<bool>) -> Config {
    let mut c = cfg(&pr.dir);
    c.allow_project_commands = allow;
    c
}

#[test]
fn cmd_9_project_relative_script_allowed() {
    let pr = proj();
    let p = policy(&proj_cfg(&pr, Some(true)), &pr.dir, &pr.f.host);
    assert_allow(&p.check_run("scripts/build.sh", &pr.dir), &pr.script);
}

#[test]
fn cmd_10_project_absolute_script_allowed() {
    let pr = proj();
    let p = policy(&proj_cfg(&pr, Some(true)), &pr.dir, &pr.f.host);
    assert_allow(
        &p.check_run(pr.script.to_str().unwrap(), &pr.dir),
        &pr.script,
    );
}

#[test]
fn cmd_11_second_root_script_allowed() {
    let f = fx();
    let a = f.root.join("root-a");
    let b = f.root.join("root-b");
    fs_err::create_dir_all(&a).unwrap();
    let script = b.join("scripts/build.sh");
    exe(&script);
    let mut c = cfg(&a);
    c.workspace_roots = Some(vec![a.to_string_lossy().into(), b.to_string_lossy().into()]);
    c.allow_project_commands = Some(true);
    let p = policy(&c, &b, &f.host);
    assert_allow(&p.check_run("scripts/build.sh", &b), &script);
}

#[test]
fn cmd_12_auto_allow_off_asks() {
    let pr = proj();
    let p = policy(&proj_cfg(&pr, Some(false)), &pr.dir, &pr.f.host);
    assert_ask(&p.check_run("scripts/build.sh", &pr.dir));
}

#[test]
fn cmd_13_multiple_all_allowed() {
    use safesh_policy::policy::check_multiple_commands;
    let (_f, p) = run_policy(&["git", "deno", "docker"]);
    let r = check_multiple_commands(
        &p,
        &[("git", "git"), ("deno", "deno"), ("docker", "docker")],
        &home(),
    );
    assert!(r.all_allowed);
    assert!(r.not_allowed.is_empty());
    assert!(r.not_found.is_empty());
}

#[test]
fn cmd_14_multiple_not_allowed() {
    use safesh_policy::policy::check_multiple_commands;
    let (_f, p) = run_policy(&["git"]);
    let r = check_multiple_commands(
        &p,
        &[("git", "git"), ("curl", "curl"), ("cargo", "cargo")],
        &home(),
    );
    assert!(!r.all_allowed);
    assert_eq!(r.not_allowed, s(&["curl", "cargo"]));
    assert!(r.results["git"].is_allow());
}

#[test]
fn cmd_15_multiple_not_found() {
    use safesh_policy::policy::check_multiple_commands;
    let f = fx();
    let c = with_run(cfg(Path::new("/nonexistent/project")), &["git"]);
    let p = policy(&c, &home(), &f.host);
    let r = check_multiple_commands(&p, &[("git", "git"), ("missing", "./missing.sh")], &home());
    assert!(!r.all_allowed);
    assert_eq!(r.not_found, s(&["./missing.sh"]));
    assert!(r.results["git"].is_allow());
}

#[test]
fn cmd_16_session_cmd_allowed() {
    let (_f, p) = session_policy(&["git"], &["curl", "cargo"]);
    assert_allow(&p.check_run("curl", &home()), Path::new("curl"));
}

#[test]
fn cmd_17_config_cmd_still_allowed_with_session() {
    let (_f, p) = session_policy(&["git"], &["curl"]);
    assert_allow(&p.check_run("git", &home()), Path::new("git"));
}

#[test]
fn cmd_18_no_session_not_allowed() {
    let (_f, p) = session_policy(&["git"], &[]);
    assert_ask(&p.check_run("curl", &home()));
}

#[test]
fn cmd_19_session_name_resolved_program_allowed() {
    let f = fx();
    let (good, bad, host) = path_host(&f, "custom-tool");
    let c = with_run(cfg(&f.root.join("project")), &["custom-tool"]);
    let p = policy(&c, &f.root, &host);
    assert_allow(&p.check_run(good.to_str().unwrap(), &f.root), &good);
    assert_ask(&p.check_run(bad.to_str().unwrap(), &f.root));
}

#[test]
fn cmd_20_session_relative_string_literal() {
    let f = fx();
    let dir = f.root.join("project");
    exe(&dir.join("packages/fyn/node_modules/.bin/mocha"));
    let mut c = with_run(cfg(&dir), &["./node_modules/.bin/mocha"]);
    c.allow_project_commands = Some(false);
    let p = policy(&c, &dir, &f.host);
    assert_allow(
        &p.check_run("./node_modules/.bin/mocha", &dir),
        Path::new("./node_modules/.bin/mocha"),
    );
}

#[test]
fn cmd_21_multiple_with_session() {
    use safesh_policy::policy::check_multiple_commands;
    let (_f, p) = session_policy(&["git"], &["cargo"]);
    let r = check_multiple_commands(
        &p,
        &[("git", "git"), ("cargo", "cargo"), ("curl", "curl")],
        &home(),
    );
    assert!(r.results["git"].is_allow());
    assert!(r.results["cargo"].is_allow());
    assert!(!r.results["curl"].is_allow());
    assert!(r.not_allowed.contains(&"curl".to_string()));
}

#[test]
fn cmd_22_session_id_defaults_auto_allow_on() {
    let pr = proj();
    let p = policy_session(&proj_cfg(&pr, None), &pr.dir, &pr.f.host);
    assert_allow(&p.check_run("scripts/build.sh", &pr.dir), &pr.script);
}

#[test]
fn cmd_23_no_session_id_auto_allow_off() {
    let pr = proj();
    let p = policy(&proj_cfg(&pr, None), &pr.dir, &pr.f.host);
    assert_ask(&p.check_run("scripts/build.sh", &pr.dir));
}

#[test]
fn cmd_24_explicit_false_beats_session_id() {
    let pr = proj();
    let p = policy_session(&proj_cfg(&pr, Some(false)), &pr.dir, &pr.f.host);
    assert_ask(&p.check_run("scripts/build.sh", &pr.dir));
}

#[test]
fn cmd_25_config_cmd_with_session_file() {
    let f = fx();
    let dir = f.root.join("project");
    let mut c = with_run(cfg(&dir), &["git"]);
    merge_session_permissions(&mut c, &dir, "sid", &f.host);
    let p = Policy::new(&c, &home(), &f.host, &PolicyOptions::default()).unwrap();
    assert_allow(&p.check_run("git", &home()), Path::new("git"));
}

#[test]
fn cmd_26_no_session_file_not_allowed() {
    let f = fx();
    let dir = f.root.join("project");
    let mut c = with_run(cfg(&dir), &["git"]);
    merge_session_permissions(&mut c, &dir, "sid", &f.host);
    let p = Policy::new(&c, &home(), &f.host, &PolicyOptions::default()).unwrap();
    assert_ask(&p.check_run("curl", &home()));
}
