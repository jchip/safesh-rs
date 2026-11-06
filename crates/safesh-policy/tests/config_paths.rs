mod config_common;

use std::path::{Path, PathBuf};

use config_common::host;
use safesh_policy::config::{
    get_global_config_dir, get_global_config_json_path, get_local_json_config_path,
    get_project_config_dir, get_project_config_json_path,
};

#[test]
fn cp_1_global_dir_default_ends_with_config_safesh() {
    let env = host(Path::new("/h"));
    assert!(get_global_config_dir(&env).ends_with(".config/safesh"));
}

#[test]
fn cp_2_global_dir_under_home() {
    let env = host(Path::new("/r"));
    assert_eq!(
        get_global_config_dir(&env),
        PathBuf::from("/r/home/.config/safesh")
    );
}

#[test]
fn cp_3_global_dir_xdg() {
    let mut env = host(Path::new("/r"));
    env.xdg_config_home = Some("/tmp/safesh-xdg".into());
    assert_eq!(
        get_global_config_dir(&env),
        PathBuf::from("/tmp/safesh-xdg/safesh")
    );
}

#[test]
fn cp_4_project_dir() {
    assert_eq!(
        get_project_config_dir(Path::new("/Users/jc/dev/safesh")),
        PathBuf::from("/Users/jc/dev/safesh/.config/safesh")
    );
}

#[test]
fn cp_5_project_dir_tmp() {
    assert_eq!(
        get_project_config_dir(Path::new("/tmp/test-project")),
        PathBuf::from("/tmp/test-project/.config/safesh")
    );
}

#[test]
fn cp_6_project_dir_relative_normalized() {
    assert_eq!(
        get_project_config_dir(Path::new("./my-project")),
        PathBuf::from("my-project/.config/safesh")
    );
}

#[test]
fn cp_9_local_json_path() {
    assert_eq!(
        get_local_json_config_path(Path::new("/Users/jc/dev/safesh")),
        PathBuf::from("/Users/jc/dev/safesh/.config/safesh/config.local.json")
    );
}

#[test]
fn cp_10_local_json_suffix() {
    for p in ["/test/project", "/any/path"] {
        let got = get_local_json_config_path(Path::new(p));
        assert!(got.ends_with("config.local.json"));
    }
}

#[test]
fn cp_12_global_json_path() {
    let env = host(Path::new("/r"));
    assert!(get_global_config_json_path(&env).ends_with(".config/safesh/config.json"));
}

#[test]
fn cp_14_project_json_path() {
    assert_eq!(
        get_project_config_json_path(Path::new("/test/project")),
        PathBuf::from("/test/project/.config/safesh/config.json")
    );
}

#[test]
fn cp_15_consistency() {
    let dir = get_project_config_dir(Path::new("/test/project"));
    assert_eq!(dir, PathBuf::from("/test/project/.config/safesh"));
    assert_eq!(
        get_local_json_config_path(Path::new("/test/project")),
        dir.join("config.local.json")
    );
    assert_eq!(
        get_project_config_json_path(Path::new("/test/project")),
        dir.join("config.json")
    );
}
