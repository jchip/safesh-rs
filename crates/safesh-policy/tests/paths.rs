use std::path::{Path, PathBuf};

use safesh_policy::host::HostEnv;
use safesh_policy::paths::*;
use tempfile::TempDir;

fn env(home: &str) -> HostEnv {
    HostEnv {
        home: PathBuf::from(home),
        tmpdir: None,
        xdg_config_home: None,
        path: None,
        claude_project_dir: None,
        uid: 502,
        state_dir: PathBuf::from("/nonexistent-state"),
    }
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

#[test]
fn pu_1_within_equal() {
    assert!(is_path_within("/foo/bar", "/foo/bar"));
}
#[test]
fn pu_2_within_child() {
    assert!(is_path_within("/foo/bar/baz", "/foo/bar"));
}
#[test]
fn pu_3_not_within_sibling() {
    assert!(!is_path_within("/foo/other", "/foo/bar"));
}
#[test]
fn pu_4_prefix_not_boundary() {
    assert!(!is_path_within("/foo/barbaz", "/foo/bar"));
}
#[test]
fn pu_5_deep_child() {
    assert!(is_path_within("/a/b/c/d/e", "/a/b"));
}
#[test]
fn pu_6_any_child() {
    assert!(is_path_within_any("/bar/subdir", &["/foo", "/bar", "/baz"]));
}
#[test]
fn pu_7_any_outside() {
    assert!(!is_path_within_any(
        "/other/path",
        &["/foo", "/bar", "/baz"]
    ));
}
#[test]
fn pu_8_any_equal() {
    assert!(is_path_within_any("/bar", &["/foo", "/bar", "/baz"]));
}
#[test]
fn pu_9_any_empty() {
    assert!(!is_path_within_any::<&str>("/any/path", &[]));
}

#[test]
fn pu_10_allowed_relative_with_cwd() {
    let t = TempDir::new().unwrap();
    let ts = s(t.path());
    assert!(is_path_within_allowed("subdir/file.txt", &[&ts], t.path()));
}
#[test]
fn pu_11_allowed_absolute() {
    let t = TempDir::new().unwrap();
    let ts = s(t.path());
    let p = format!("{ts}/test");
    assert!(is_path_within_allowed(&p, &[&ts], Path::new("/")));
}
#[test]
fn pu_12_allowed_outside() {
    let t = TempDir::new().unwrap();
    let ts = s(t.path());
    assert!(!is_path_within_allowed(
        "/outside/path",
        &[&ts],
        Path::new("/")
    ));
}

#[test]
fn pd_1_project_child() {
    assert!(is_within_project_dir(
        "/project/src/file.ts",
        "/project",
        Path::new("/")
    ));
}
#[test]
fn pd_2_project_equal() {
    assert!(is_within_project_dir(
        "/project",
        "/project",
        Path::new("/")
    ));
}
#[test]
fn pd_3_project_other() {
    assert!(!is_within_project_dir(
        "/other/file.ts",
        "/project",
        Path::new("/")
    ));
}
#[test]
fn pd_4_project_prefix() {
    assert!(!is_within_project_dir("/proj", "/project", Path::new("/")));
}
#[test]
fn pd_5_project_relative() {
    assert!(is_within_project_dir(
        "./src/file.ts",
        "/project",
        Path::new("/project")
    ));
}
#[test]
fn pd_6_project_relative_escape() {
    assert!(!is_within_project_dir(
        "../other/file.ts",
        "/project",
        Path::new("/project")
    ));
}
#[test]
fn pd_7_command_relative() {
    assert!(is_command_within_project_dir(
        "./scripts/build.sh",
        "/project",
        Path::new("/project")
    ));
}
#[test]
fn pd_8_command_absolute() {
    assert!(is_command_within_project_dir(
        "/project/scripts/build.sh",
        "/project",
        Path::new("/")
    ));
}
#[test]
fn pd_9_command_bare_name() {
    assert!(!is_command_within_project_dir(
        "git",
        "/project",
        Path::new("/")
    ));
}
#[test]
fn pd_10_command_outside() {
    assert!(!is_command_within_project_dir(
        "/other/scripts/build.sh",
        "/project",
        Path::new("/")
    ));
}

#[test]
fn tp_1_expand_braced_cwd() {
    let e = env("/h");
    assert_eq!(
        expand_path("${CWD}/src", Path::new("/my/project"), None, &e),
        "/my/project/src"
    );
}
#[test]
fn tp_2_expand_bare_cwd() {
    let e = env("/h");
    assert_eq!(
        expand_path("$CWD/src", Path::new("/my/project"), None, &e),
        "/my/project/src"
    );
}
#[test]
fn tp_3_expand_home() {
    let e = env("/Users/test-home");
    assert_eq!(
        expand_path("${HOME}/.config", Path::new("/cwd"), None, &e),
        "/Users/test-home/.config"
    );
}
#[test]
fn tp_4_expand_no_slash_normalization() {
    let e = env("/Users/test-home");
    assert_eq!(
        expand_path(
            "${HOME}/projects/${CWD}/file",
            Path::new("/myproj"),
            None,
            &e
        ),
        "/Users/test-home/projects//myproj/file"
    );
}
#[test]
fn tp_expand_tilde_workspace_and_word_boundary() {
    let e = env("/h");
    let c = Path::new("/c");
    assert_eq!(expand_path("~", c, None, &e), "/h");
    assert_eq!(expand_path("~/x", c, None, &e), "/h/x");
    assert_eq!(expand_path("$WORKSPACE/x", c, Some("/w"), &e), "/w/x");
    assert_eq!(expand_path("$CWDX/x", c, None, &e), "$CWDX/x");
}

fn allowed() -> [&'static str; 1] {
    ["/tmp/safesh-test"]
}
#[test]
fn tp_6_allowed_equal() {
    assert!(is_path_allowed(
        "/tmp/safesh-test",
        &allowed(),
        Path::new("/"),
        None,
        &env("/h")
    ));
}
#[test]
fn tp_7_allowed_child() {
    assert!(is_path_allowed(
        "/tmp/safesh-test/subdir/file.txt",
        &allowed(),
        Path::new("/"),
        None,
        &env("/h")
    ));
}
#[test]
fn tp_8_allowed_parent() {
    assert!(!is_path_allowed(
        "/tmp",
        &allowed(),
        Path::new("/"),
        None,
        &env("/h")
    ));
}
#[test]
fn tp_9_allowed_sibling() {
    assert!(!is_path_allowed(
        "/tmp/other",
        &allowed(),
        Path::new("/"),
        None,
        &env("/h")
    ));
}
#[test]
fn tp_10_allowed_dotdot() {
    assert!(!is_path_allowed(
        "/tmp/safesh-test/../other",
        &allowed(),
        Path::new("/"),
        None,
        &env("/h")
    ));
}
#[test]
fn tp_11_allowed_relative() {
    assert!(is_path_allowed(
        "src/file.ts",
        &["/project"],
        Path::new("/project"),
        None,
        &env("/h")
    ));
}

#[test]
fn perm_1_temp_strips_trailing_slash() {
    let r = compute_temp_dir_defaults(Some("/var/folders/0p/abc123/T/"));
    assert!(r.contains(&"/var/folders/0p/abc123/T".to_string()));
    assert!(r.contains(&"/tmp".to_string()));
}
#[test]
fn perm_2_temp_includes_real_path() {
    let t = TempDir::new().unwrap();
    let lit = s(t.path());
    let real = s(&std::fs::canonicalize(t.path()).unwrap());
    let r = compute_temp_dir_defaults(Some(&format!("{lit}/")));
    assert!(r.contains(&lit));
    assert!(r.contains(&real));
}
#[test]
fn perm_3_temp_slash_tmp_once() {
    let r = compute_temp_dir_defaults(Some("/tmp/"));
    assert_eq!(r.iter().filter(|d| *d == "/tmp").count(), 1);
}
#[test]
fn perm_4_temp_none() {
    let r = compute_temp_dir_defaults(None);
    assert!(r.contains(&"/tmp".to_string()));
    assert!(!r.iter().any(|d| d.starts_with("/var/folders")));
}
#[test]
fn perm_7_path_dirs() {
    let r = compute_path_dir_defaults(Some("/usr/bin:/bin:/usr/bin:/opt/x/:.::relative".as_ref()));
    assert_eq!(r, ["/usr/bin", "/bin", "/opt/x"]);
}
#[test]
fn perm_8_path_dirs_empty() {
    assert!(compute_path_dir_defaults(None).is_empty());
    assert!(compute_path_dir_defaults(Some("".as_ref())).is_empty());
}
#[test]
fn perm_11_home_dot_dirs() {
    let h = TempDir::new().unwrap();
    for d in [".cache", ".config", "visible"] {
        std::fs::create_dir(h.path().join(d)).unwrap();
    }
    std::fs::write(h.path().join(".zshrc"), "").unwrap();
    std::os::unix::fs::symlink("/etc", h.path().join(".link")).unwrap();
    let r = compute_home_dot_dir_defaults(Some(h.path()));
    assert_eq!(
        r,
        [s(&h.path().join(".cache")), s(&h.path().join(".config"))]
    );
}
#[test]
fn perm_12_home_dot_dirs_invalid() {
    assert!(compute_home_dot_dir_defaults(None).is_empty());
    assert!(compute_home_dot_dir_defaults(Some(Path::new("relative"))).is_empty());
    assert!(compute_home_dot_dir_defaults(Some(Path::new("/nonexistent-safesh-home"))).is_empty());
}

#[test]
fn resolve_existing_file() {
    let t = TempDir::new().unwrap();
    let real = std::fs::canonicalize(t.path()).unwrap();
    std::fs::write(real.join("f.txt"), "x").unwrap();
    let r = resolve_path(&t.path().join("f.txt"), Path::new("/"));
    assert_eq!(r, real.join("f.txt"));
}
#[test]
fn resolve_missing_file_in_existing_dir() {
    let t = TempDir::new().unwrap();
    let real = std::fs::canonicalize(t.path()).unwrap();
    let r = resolve_path(&t.path().join("new/deep/file.txt"), Path::new("/"));
    assert_eq!(r, real.join("new/deep/file.txt"));
}
#[test]
fn resolve_symlink_escaping_dir() {
    let t = TempDir::new().unwrap();
    let real = std::fs::canonicalize(t.path()).unwrap();
    let outside = TempDir::new().unwrap();
    let outside_real = std::fs::canonicalize(outside.path()).unwrap();
    std::fs::write(outside_real.join("secret"), "x").unwrap();
    std::os::unix::fs::symlink(&outside_real, real.join("link")).unwrap();
    let r = resolve_path(&real.join("link/secret"), Path::new("/"));
    assert_eq!(r, outside_real.join("secret"));
    assert!(!is_path_within(&s(&r), &s(&real)));
    // A missing file behind the symlinked dir also escapes.
    let r2 = resolve_path(&real.join("link/missing"), Path::new("/"));
    assert_eq!(r2, outside_real.join("missing"));
}
#[test]
fn resolve_relative_with_dotdot() {
    let t = TempDir::new().unwrap();
    let real = std::fs::canonicalize(t.path()).unwrap();
    std::fs::create_dir(real.join("a")).unwrap();
    let r = resolve_path(Path::new("../a/./x.txt"), &real.join("a"));
    assert_eq!(r, real.join("a/x.txt"));
}
#[test]
fn resolve_dotdot_past_root_and_private_tmp_fold() {
    assert_eq!(
        resolve_path(Path::new("/../../nonexistent-safesh-x"), Path::new("/")),
        Path::new("/nonexistent-safesh-x")
    );
    let r = resolve_path(
        Path::new("/private/tmp/nonexistent-safesh-x/f"),
        Path::new("/"),
    );
    assert_eq!(r, Path::new("/tmp/nonexistent-safesh-x/f"));
}
