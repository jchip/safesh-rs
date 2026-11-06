#![allow(clippy::needless_pass_by_value)]

mod config_common;

use std::path::Path;

use config_common::{host, s, write};
use safesh_policy::config::{
    ConfigUpdate, UpdateOptions, add_commands_to_config, add_paths_to_config,
    get_global_config_json_path, get_local_json_config_path, save_to_global_json,
    save_to_local_json, update_config_local,
};
use serde_json::{Value, json};
use tempfile::TempDir;

fn read(path: &Path) -> Value {
    serde_json::from_str(&fs_err::read_to_string(path).unwrap()).unwrap()
}

fn upd(dir: &Path, u: ConfigUpdate) {
    update_config_local(dir, &u, UpdateOptions::default()).unwrap();
}

fn cmds(c: &[&str]) -> ConfigUpdate {
    ConfigUpdate {
        commands: Some(s(c)),
        ..ConfigUpdate::default()
    }
}

#[test]
fn cpr_1_creates_file() {
    let t = TempDir::new().unwrap();
    upd(t.path(), cmds(&["ls"]));
    assert_eq!(
        read(&get_local_json_config_path(t.path()))["allowedCommands"],
        json!(["ls"])
    );
}

#[test]
fn cpr_2_merges() {
    let t = TempDir::new().unwrap();
    upd(t.path(), cmds(&["ls"]));
    upd(t.path(), cmds(&["cat", "grep"]));
    assert_eq!(
        read(&get_local_json_config_path(t.path()))["allowedCommands"],
        json!(["ls", "cat", "grep"])
    );
}

#[test]
fn cpr_3_dedupes() {
    let t = TempDir::new().unwrap();
    upd(t.path(), cmds(&["ls"]));
    upd(t.path(), cmds(&["ls"]));
    assert_eq!(
        read(&get_local_json_config_path(t.path()))["allowedCommands"],
        json!(["ls"])
    );
}

#[test]
fn cpr_4_read_paths() {
    let t = TempDir::new().unwrap();
    upd(
        t.path(),
        ConfigUpdate {
            read_paths: Some(s(&["/tmp", "/var"])),
            ..Default::default()
        },
    );
    assert_eq!(
        read(&get_local_json_config_path(t.path()))["permissions"]["read"],
        json!(["/tmp", "/var"])
    );
}

#[test]
fn cpr_5_write_paths() {
    let t = TempDir::new().unwrap();
    upd(
        t.path(),
        ConfigUpdate {
            write_paths: Some(s(&["/tmp"])),
            ..Default::default()
        },
    );
    assert_eq!(
        read(&get_local_json_config_path(t.path()))["permissions"]["write"],
        json!(["/tmp"])
    );
}

#[test]
fn cpr_6_read_then_read_and_write() {
    let t = TempDir::new().unwrap();
    upd(
        t.path(),
        ConfigUpdate {
            read_paths: Some(s(&["/tmp"])),
            ..Default::default()
        },
    );
    upd(
        t.path(),
        ConfigUpdate {
            read_paths: Some(s(&["/var"])),
            write_paths: Some(s(&["/tmp"])),
            ..Default::default()
        },
    );
    let v = read(&get_local_json_config_path(t.path()));
    assert_eq!(v["permissions"]["read"], json!(["/tmp", "/var"]));
    assert_eq!(v["permissions"]["write"], json!(["/tmp"]));
}

#[test]
fn cpr_7_all_three() {
    let t = TempDir::new().unwrap();
    upd(
        t.path(),
        ConfigUpdate {
            commands: Some(s(&["ls", "cat"])),
            read_paths: Some(s(&["/tmp"])),
            write_paths: Some(s(&["/var"])),
        },
    );
    let v = read(&get_local_json_config_path(t.path()));
    assert_eq!(v["allowedCommands"], json!(["ls", "cat"]));
    assert_eq!(v["permissions"]["read"], json!(["/tmp"]));
    assert_eq!(v["permissions"]["write"], json!(["/var"]));
}

#[test]
fn cpr_8_replace() {
    let t = TempDir::new().unwrap();
    upd(t.path(), cmds(&["ls", "cat"]));
    let msgs =
        update_config_local(t.path(), &cmds(&["grep"]), UpdateOptions { merge: false }).unwrap();
    assert_eq!(msgs, vec!["commands: grep".to_string()]);
    assert_eq!(
        read(&get_local_json_config_path(t.path()))["allowedCommands"],
        json!(["grep"])
    );
}

#[test]
fn cpr_9_add_commands() {
    let t = TempDir::new().unwrap();
    add_commands_to_config(&s(&["ls", "cat"]), t.path()).unwrap();
    let v = read(&get_local_json_config_path(t.path()));
    assert_eq!(v["allowedCommands"].as_array().unwrap().len(), 2);
}

#[test]
fn cpr_10_add_paths() {
    let t = TempDir::new().unwrap();
    add_paths_to_config(&s(&["/tmp"]), &s(&["/var"]), t.path()).unwrap();
    let v = read(&get_local_json_config_path(t.path()));
    assert_eq!(v["permissions"]["read"], json!(["/tmp"]));
    assert_eq!(v["permissions"]["write"], json!(["/var"]));
}

#[test]
fn update_keeps_unrelated_keys() {
    let t = TempDir::new().unwrap();
    write(&get_local_json_config_path(t.path()), r#"{"other":1}"#);
    upd(t.path(), cmds(&["ls"]));
    assert_eq!(
        read(&get_local_json_config_path(t.path()))["other"],
        json!(1)
    );
}

#[test]
fn cl_16_save_local() {
    let t = TempDir::new().unwrap();
    save_to_local_json(t.path(), &s(&["cargo", "rustc"])).unwrap();
    assert_eq!(
        read(&get_local_json_config_path(t.path()))["allowedCommands"],
        json!(["cargo", "rustc"])
    );
}

#[test]
fn cl_17_save_local_sorted_merge() {
    let t = TempDir::new().unwrap();
    save_to_local_json(t.path(), &s(&["cargo"])).unwrap();
    save_to_local_json(t.path(), &s(&["rustc", "make"])).unwrap();
    assert_eq!(
        read(&get_local_json_config_path(t.path()))["allowedCommands"],
        json!(["cargo", "make", "rustc"])
    );
}

#[test]
fn cl_18_save_local_dedup_sort() {
    let t = TempDir::new().unwrap();
    save_to_local_json(t.path(), &s(&["cargo", "rustc"])).unwrap();
    save_to_local_json(t.path(), &s(&["cargo", "make"])).unwrap();
    assert_eq!(
        read(&get_local_json_config_path(t.path()))["allowedCommands"],
        json!(["cargo", "make", "rustc"])
    );
}

#[test]
fn cl_23_save_local_creates_dir() {
    let t = TempDir::new().unwrap();
    save_to_local_json(t.path(), &s(&["cargo"])).unwrap();
    assert_eq!(
        read(&get_local_json_config_path(t.path()))["allowedCommands"],
        json!(["cargo"])
    );
}

#[test]
fn cl_24_save_global_creates_run_list() {
    let t = TempDir::new().unwrap();
    let mut env = host(t.path());
    env.xdg_config_home = Some(t.path().join("xdg"));
    save_to_global_json(&env, &s(&["curl", "wget"])).unwrap();
    let p = get_global_config_json_path(&env);
    assert!(p.starts_with(t.path().join("xdg/safesh")));
    assert_eq!(read(&p)["permissions"]["run"], json!(["curl", "wget"]));
}

#[test]
fn cl_25_save_global_keeps_existing() {
    let t = TempDir::new().unwrap();
    let mut env = host(t.path());
    env.xdg_config_home = Some(t.path().join("xdg"));
    let p = get_global_config_json_path(&env);
    write(
        &p,
        r#"{"workspaceDir":"~/workspace","permissions":{"run":["gh"],"read":["/tmp"]},"external":{"gh":{"allow":true}}}"#,
    );
    save_to_global_json(&env, &s(&["docker"])).unwrap();
    let v = read(&p);
    assert_eq!(v["permissions"]["run"], json!(["docker", "gh"]));
    assert_eq!(v["permissions"]["read"], json!(["/tmp"]));
    assert_eq!(v["workspaceDir"], json!("~/workspace"));
    assert_eq!(v["external"]["gh"], json!({"allow": true}));
}

#[test]
fn written_file_is_two_space_json_with_newline() {
    let t = TempDir::new().unwrap();
    save_to_local_json(t.path(), &s(&["a"])).unwrap();
    let text = fs_err::read_to_string(get_local_json_config_path(t.path())).unwrap();
    assert_eq!(text, "{\n  \"allowedCommands\": [\n    \"a\"\n  ]\n}\n");
}
