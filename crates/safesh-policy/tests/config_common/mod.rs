#![allow(dead_code)]

use std::path::Path;

use safesh_policy::host::HostEnv;

/// Host env rooted in `root`: HOME, no XDG, no TMPDIR.
pub fn host(root: &Path) -> HostEnv {
    HostEnv {
        home: root.join("home"),
        tmpdir: None,
        xdg_config_home: None,
        path: None,
        claude_project_dir: None,
        uid: 0,
        state_dir: root.join("state"),
    }
}

pub fn write(path: &Path, text: &str) {
    fs_err::create_dir_all(path.parent().unwrap()).unwrap();
    fs_err::write(path, text).unwrap();
}

pub fn s(items: &[&str]) -> Vec<String> {
    items.iter().map(|x| (*x).to_string()).collect()
}
