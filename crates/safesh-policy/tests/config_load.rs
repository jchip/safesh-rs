mod config_common;

use config_common::{host, s, write};
use safesh_policy::config::{
    ConfigError, NetPermission, get_global_config_json_path, get_local_json_config_path,
    get_project_config_json_path, load_config,
};
use tempfile::TempDir;

fn run_of(c: &safesh_policy::config::Config) -> Vec<String> {
    c.permissions.clone().unwrap().run.unwrap()
}

#[test]
fn cl_5_no_files_gives_defaults() {
    let t = TempDir::new().unwrap();
    let env = host(t.path());
    let out = load_config(t.path(), None, &env).unwrap();
    assert!(out.config.permissions.is_some());
    assert!(run_of(&out.config).contains(&"git".to_string()));
}

#[test]
fn cl_6_global_workspace_dir_sets_alias() {
    let t = TempDir::new().unwrap();
    let mut env = host(t.path());
    env.xdg_config_home = Some(t.path().join("xdg"));
    let w = t.path().join("workspace");
    write(
        &get_global_config_json_path(&env),
        &format!(r#"{{"workspaceDir":"{}"}}"#, w.display()),
    );
    let c = load_config(t.path(), None, &env).unwrap().config;
    assert_eq!(c.workspace_dir.as_deref(), w.to_str());
    assert_eq!(c.workspace.as_deref(), w.to_str());
}

#[test]
fn cl_9_project_and_local_merge() {
    let t = TempDir::new().unwrap();
    let env = host(t.path());
    write(
        &get_project_config_json_path(t.path()),
        r#"{"permissions":{"run":["git2"]}}"#,
    );
    write(
        &get_local_json_config_path(t.path()),
        r#"{"allowedCommands":["cargo2"]}"#,
    );
    let run = run_of(&load_config(t.path(), None, &env).unwrap().config);
    assert!(run.contains(&"git2".into()) && run.contains(&"cargo2".into()));
}

#[test]
fn cl_12_project_run_plus_local() {
    let t = TempDir::new().unwrap();
    let env = host(t.path());
    write(
        &get_project_config_json_path(t.path()),
        r#"{"permissions":{"run":["git","docker"]}}"#,
    );
    write(
        &get_local_json_config_path(t.path()),
        r#"{"allowedCommands":["cargo"]}"#,
    );
    let run = run_of(&load_config(t.path(), None, &env).unwrap().config);
    for c in ["git", "docker", "cargo"] {
        assert!(run.contains(&c.to_string()));
    }
}

#[test]
fn cl_3_and_14_mirror_local_object_entries() {
    let t = TempDir::new().unwrap();
    let env = host(t.path());
    write(
        &get_local_json_config_path(t.path()),
        r#"{"allowedCommands":["mk",{"command":"gitx","subcommands":["status","log"]},{"name":"build","path":"./scripts/build.sh"}]}"#,
    );
    let out = load_config(t.path(), None, &env).unwrap();
    let run = run_of(&out.config);
    for c in ["mk", "gitx", "./scripts/build.sh"] {
        assert!(run.contains(&c.to_string()), "missing {c}");
    }
    assert!(
        out.warnings
            .iter()
            .any(|w| w.contains("subcommand limits for 'gitx'"))
    );
}

#[test]
fn cl_11_and_19_local_json_commands() {
    let t = TempDir::new().unwrap();
    let env = host(t.path());
    write(
        &get_local_json_config_path(t.path()),
        r#"{"allowedCommands":["cargo2","rustc2","make2"]}"#,
    );
    let run = run_of(&load_config(t.path(), None, &env).unwrap().config);
    for c in ["cargo2", "rustc2", "make2"] {
        assert!(run.contains(&c.to_string()));
    }
}

#[test]
fn cl_21_bad_local_json_only_warns() {
    let t = TempDir::new().unwrap();
    let env = host(t.path());
    write(&get_local_json_config_path(t.path()), "{ invalid json here");
    let out = load_config(t.path(), None, &env).unwrap();
    assert!(out.config.permissions.is_some());
    assert!(
        out.warnings
            .iter()
            .any(|w| w.contains("failed to load local config"))
    );
}

#[test]
fn cl_22_empty_local_json_gives_defaults() {
    let t = TempDir::new().unwrap();
    let env = host(t.path());
    write(&get_local_json_config_path(t.path()), "{}");
    let out = load_config(t.path(), None, &env).unwrap();
    assert!(out.config.permissions.is_some());
}

#[test]
fn cl_26_global_saved_commands_reach_every_project() {
    let t = TempDir::new().unwrap();
    let mut env = host(t.path());
    env.xdg_config_home = Some(t.path().join("xdg"));
    safesh_policy::config::save_to_global_json(&env, &s(&["rsync"])).unwrap();
    for d in ["p1", "p2"] {
        let dir = t.path().join(d);
        let run = run_of(&load_config(&dir, None, &env).unwrap().config);
        assert!(run.contains(&"rsync".to_string()));
    }
}

#[test]
fn project_dir_overrides_cwd_for_project_files() {
    let t = TempDir::new().unwrap();
    let env = host(t.path());
    let proj = t.path().join("proj");
    let cwd = proj.join("sub");
    write(
        &get_project_config_json_path(&proj),
        r#"{"permissions":{"run":["only-in-proj"]}}"#,
    );
    let with = load_config(&cwd, Some(&proj), &env).unwrap().config;
    assert!(run_of(&with).contains(&"only-in-proj".to_string()));
    let without = load_config(&cwd, None, &env).unwrap().config;
    assert!(!run_of(&without).contains(&"only-in-proj".to_string()));
}

#[test]
fn paths_stay_unexpanded() {
    let t = TempDir::new().unwrap();
    let env = host(t.path());
    write(
        &get_project_config_json_path(t.path()),
        r#"{"workspaceDir":"~/ws","workspaceRoots":["${HOME}/a"]}"#,
    );
    let c = load_config(t.path(), None, &env).unwrap().config;
    assert_eq!(c.workspace_dir.as_deref(), Some("~/ws"));
    assert_eq!(c.workspace.as_deref(), Some("~/ws"));
    assert_eq!(c.workspace_roots, Some(s(&["${HOME}/a"])));
}

#[test]
fn net_accepts_bool_or_list() {
    let t = TempDir::new().unwrap();
    let env = host(t.path());
    write(
        &get_project_config_json_path(t.path()),
        r#"{"permissions":{"net":["a.com"]}}"#,
    );
    let c = load_config(t.path(), None, &env).unwrap().config;
    assert_eq!(
        c.permissions.unwrap().net,
        Some(NetPermission::Hosts(s(&["a.com"])))
    );
}

#[test]
fn dropped_and_unknown_keys_warn_but_load() {
    let t = TempDir::new().unwrap();
    let env = host(t.path());
    write(
        &get_project_config_json_path(t.path()),
        r#"{"imports":{},"denoFlags":[],"vfs":{},"nativeCommands":true,"projectTemp":true,"alwaysTranspile":true,"tasks":{},"external":{},"mystery":1}"#,
    );
    let out = load_config(t.path(), None, &env).unwrap();
    for k in [
        "imports",
        "denoFlags",
        "vfs",
        "nativeCommands",
        "projectTemp",
        "alwaysTranspile",
        "tasks",
        "external",
    ] {
        let needle = format!("'{k}' is no longer supported");
        assert!(
            out.warnings.iter().any(|w| w.contains(&needle)),
            "no warning for {k}"
        );
    }
    assert!(
        out.warnings
            .iter()
            .any(|w| w.contains("unknown key 'mystery'"))
    );
}

#[test]
fn parse_error_names_field_path() {
    let t = TempDir::new().unwrap();
    let env = host(t.path());
    write(
        &get_project_config_json_path(t.path()),
        r#"{"permissions":{"read":"nope"}}"#,
    );
    let err = load_config(t.path(), None, &env).unwrap_err();
    assert!(matches!(err, ConfigError::Parse { .. }));
    assert!(err.to_string().contains("permissions.read"), "{err}");
}

#[test]
fn validation_error_fails_load() {
    let t = TempDir::new().unwrap();
    let env = host(t.path());
    write(
        &get_project_config_json_path(t.path()),
        r#"{"permissions":{"write":["/"]}}"#,
    );
    let err = load_config(t.path(), None, &env).unwrap_err();
    assert!(matches!(err, ConfigError::Validation(_)));
}

#[test]
fn ts_config_without_json_warns_migrate() {
    let t = TempDir::new().unwrap();
    let mut env = host(t.path());
    env.xdg_config_home = Some(t.path().join("xdg"));
    let dir = t.path().join(".config/safesh");
    write(&dir.join("config.ts"), "export default {}");
    write(&dir.join("config.local.ts"), "export default {}");
    write(&t.path().join("xdg/safesh/config.ts"), "export default {}");
    let out = load_config(t.path(), None, &env).unwrap();
    let n = out
        .warnings
        .iter()
        .filter(|w| w.contains("desh migrate-config"))
        .count();
    assert_eq!(n, 3);
}

#[test]
fn ts_config_with_json_beside_does_not_warn() {
    let t = TempDir::new().unwrap();
    let env = host(t.path());
    let dir = t.path().join(".config/safesh");
    write(&dir.join("config.ts"), "export default {}");
    write(&dir.join("config.json"), "{}");
    let out = load_config(t.path(), None, &env).unwrap();
    assert!(!out.warnings.iter().any(|w| w.contains("migrate-config")));
}
