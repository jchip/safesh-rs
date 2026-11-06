#![allow(clippy::unnecessary_wraps)]

use safesh_policy::config::{
    Config, NetPermission, PermissionsConfig, default_config, validate_config,
};

fn perms(p: PermissionsConfig) -> Config {
    Config {
        permissions: Some(p),
        ..Config::default()
    }
}

fn v(items: &[&str]) -> Option<Vec<String>> {
    Some(items.iter().map(|s| (*s).to_string()).collect())
}

fn any(list: &[String], subs: &[&str]) -> bool {
    list.iter().any(|m| subs.iter().all(|s| m.contains(s)))
}

#[test]
fn cv_1_default_static_lists() {
    let p = default_config().permissions.unwrap();
    assert_eq!(p.read, v(&["${CWD}", "${HOME}", "/tmp", "${HOME}/.claude"]));
    // Dynamic home dot-dirs are appended by policy, so only the static prefix is here.
    assert_eq!(p.write, v(&["/tmp", "/dev/null", "${HOME}/.claude"]));
    assert_eq!(p.net, Some(NetPermission::Hosts(vec![])));
}

#[test]
fn cv_2_to_6_default_run_has_commands() {
    let run = default_config().permissions.unwrap().run.unwrap();
    for c in [
        "python3", "grep", "rg", "find", "fd", "swift", "desh", "tmux",
    ] {
        assert!(run.iter().any(|x| x == c), "missing {c}");
    }
}

#[test]
fn cv_8_default_has_no_errors() {
    assert!(validate_config(&default_config()).errors.is_empty());
}

#[test]
fn cv_9_write_root_is_error() {
    let r = validate_config(&perms(PermissionsConfig {
        write: v(&["/"]),
        ..Default::default()
    }));
    assert!(any(&r.errors, &["write", "/"]));
}

#[test]
fn cv_10_run_star_is_error() {
    let r = validate_config(&perms(PermissionsConfig {
        run: v(&["*"]),
        ..Default::default()
    }));
    assert!(any(&r.errors, &["run", "*"]));
}

#[test]
fn cv_11_read_root_is_warning() {
    let r = validate_config(&perms(PermissionsConfig {
        read: v(&["/"]),
        ..Default::default()
    }));
    assert!(r.errors.is_empty());
    assert!(any(&r.warnings, &["read", "/"]));
}

#[test]
fn cv_12_read_sensitive_dirs_warn() {
    let r = validate_config(&perms(PermissionsConfig {
        read: v(&["/etc", "/var"]),
        ..Default::default()
    }));
    assert!(any(&r.warnings, &["/etc"]));
    assert!(any(&r.warnings, &["/var"]));
}

#[test]
fn cv_13_write_dangerous_dirs_error() {
    let r = validate_config(&perms(PermissionsConfig {
        write: v(&["/etc", "/bin"]),
        ..Default::default()
    }));
    assert!(any(&r.errors, &["/etc"]));
    assert!(any(&r.errors, &["/bin"]));
}

#[test]
fn cv_14_net_true_warns() {
    let r = validate_config(&perms(PermissionsConfig {
        net: Some(NetPermission::All(true)),
        ..Default::default()
    }));
    assert!(any(&r.warnings, &["unrestricted network"]));
}

#[test]
fn cv_15_many_commands_no_warning() {
    let run: Vec<String> = (0..25).map(|i| format!("cmd{i}")).collect();
    let r = validate_config(&perms(PermissionsConfig {
        run: Some(run),
        ..Default::default()
    }));
    assert!(!any(&r.warnings, &["25 commands"]));
}

#[test]
fn cvs_1_read_cwd_tmp_ok() {
    let r = validate_config(&perms(PermissionsConfig {
        read: v(&["${CWD}", "/tmp"]),
        ..Default::default()
    }));
    assert!(r.errors.is_empty());
}

#[test]
fn cvs_6_net_hosts_no_warning() {
    let r = validate_config(&perms(PermissionsConfig {
        net: Some(NetPermission::Hosts(vec!["example.com".into()])),
        ..Default::default()
    }));
    assert!(r.errors.is_empty());
    assert!(!any(&r.warnings, &["unrestricted network"]));
}

#[test]
fn cvs_15_unset_project_warns() {
    let r = validate_config(&Config::default());
    assert!(any(&r.warnings, &["projectDir/workspaceRoots: not set"]));
}

#[test]
fn cvs_16_workspace_roots_no_warning() {
    let r = validate_config(&Config {
        workspace_roots: v(&["/home/user/project-a", "/home/user/project-b"]),
        ..Config::default()
    });
    assert!(!any(&r.warnings, &["projectDir/workspaceRoots: not set"]));
}

#[test]
fn cvs_17_workspace_dir_no_warning() {
    let r = validate_config(&Config {
        workspace_dir: Some("/home/user".into()),
        ..Config::default()
    });
    assert!(!any(&r.warnings, &["projectDir/workspaceRoots: not set"]));
}

#[test]
fn cvs_18_project_dir_no_warning() {
    let r = validate_config(&Config {
        project_dir: Some("/home/user/project".into()),
        ..Config::default()
    });
    assert!(!any(&r.warnings, &["projectDir/workspaceRoots: not set"]));
}

#[test]
fn cvs_21_multiple_errors() {
    let r = validate_config(&perms(PermissionsConfig {
        write: v(&["/"]),
        run: v(&["*"]),
        ..Default::default()
    }));
    assert!(r.errors.len() >= 2);
    assert!(any(&r.errors, &["write", "/"]));
    assert!(any(&r.errors, &["run", "*"]));
}

#[test]
fn cvs_22_empty_config_warns_project_dir() {
    let r = validate_config(&Config::default());
    assert!(r.errors.is_empty());
    assert!(any(&r.warnings, &["projectDir"]));
}
