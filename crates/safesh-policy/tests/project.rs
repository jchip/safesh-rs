use std::fs;
use std::path::{Path, PathBuf};

use safesh_policy::host::HostEnv;
use safesh_policy::project::*;
use tempfile::TempDir;

fn env(home: &Path) -> HostEnv {
    HostEnv {
        home: home.to_path_buf(),
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

fn no_cfg() -> FindProjectRootOptions {
    FindProjectRootOptions {
        create_config: false,
        stop_at_home: true,
    }
}

// Tests use a fake HOME inside the tmp dir so ambient HOME never matters.
fn fixture() -> (TempDir, PathBuf, HostEnv) {
    let t = TempDir::new().unwrap();
    let base = t.path().to_path_buf();
    let e = env(&base.join("fake-home"));
    (t, base, e)
}

#[test]
fn pr_1_env_wins() {
    let (_t, base, mut e) = fixture();
    e.claude_project_dir = Some(PathBuf::from("/custom/project/dir"));
    assert_eq!(
        find_project_root(&base, FindProjectRootOptions::default(), &e),
        Path::new("/custom/project/dir")
    );
}
#[test]
fn pr_2_claude_marker() {
    let (_t, b, e) = fixture();
    let sub = b.join("my-project/src/components");
    fs::create_dir_all(&sub).unwrap();
    fs::create_dir(b.join("my-project/.claude")).unwrap();
    assert_eq!(find_project_root(&sub, no_cfg(), &e), b.join("my-project"));
}
#[test]
fn pr_3_git_marker() {
    let (_t, b, e) = fixture();
    let sub = b.join("my-repo/src/utils");
    fs::create_dir_all(&sub).unwrap();
    fs::create_dir(b.join("my-repo/.git")).unwrap();
    assert_eq!(find_project_root(&sub, no_cfg(), &e), b.join("my-repo"));
}
#[test]
fn pr_4_safesh_marker() {
    let (_t, b, e) = fixture();
    let sub = b.join("safesh-project/lib/core");
    fs::create_dir_all(&sub).unwrap();
    fs::create_dir_all(b.join("safesh-project/.config/safesh")).unwrap();
    assert_eq!(
        find_project_root(&sub, no_cfg(), &e),
        b.join("safesh-project")
    );
}
#[test]
fn pr_5_stops_at_home() {
    let (_t, b, e) = fixture();
    let sub = b.join("fake-home/workspace/project");
    fs::create_dir_all(&sub).unwrap();
    fs::create_dir(b.join("fake-home/.git")).unwrap();
    assert_eq!(find_project_root(&sub, no_cfg(), &e), sub);
}
#[test]
fn pr_6_creates_config() {
    let (_t, b, e) = fixture();
    let d = b.join("new-project");
    fs::create_dir_all(&d).unwrap();
    assert_eq!(
        find_project_root(&d, FindProjectRootOptions::default(), &e),
        d
    );
    assert_eq!(
        fs::read_to_string(d.join(".config/safesh/config.local.json")).unwrap(),
        "{}\n"
    );
}
#[test]
fn pr_7_no_config_created() {
    let (_t, b, e) = fixture();
    let d = b.join("no-config-project");
    fs::create_dir_all(&d).unwrap();
    assert_eq!(find_project_root(&d, no_cfg(), &e), d);
    assert!(!d.join(".config/safesh").exists());
}
#[test]
fn pr_8_cwd_when_no_marker() {
    let (_t, b, e) = fixture();
    let d = b.join("bare-project");
    fs::create_dir_all(&d).unwrap();
    assert_eq!(find_project_root(&d, no_cfg(), &e), d);
}
#[test]
fn pr_9_first_marker_upward() {
    let (_t, b, e) = fixture();
    let sub = b.join("outer-project/inner-project/src");
    fs::create_dir_all(&sub).unwrap();
    fs::create_dir(b.join("outer-project/.git")).unwrap();
    fs::create_dir(b.join("outer-project/inner-project/.git")).unwrap();
    assert_eq!(
        find_project_root(&sub, no_cfg(), &e),
        b.join("outer-project/inner-project")
    );
}
#[test]
fn pr_10_nested_clone_does_not_take_over() {
    let (_t, b, e) = fixture();
    let sub = b.join("project/.temp/clone/src");
    fs::create_dir_all(&sub).unwrap();
    fs::create_dir(b.join("project/.claude")).unwrap();
    fs::create_dir(b.join("project/.git")).unwrap();
    fs::create_dir(b.join("project/.temp/clone/.git")).unwrap();
    assert_eq!(find_project_root(&sub, no_cfg(), &e), b.join("project"));
}
#[test]
fn pr_11_conventional_worktree_maps_to_parent() {
    let (_t, b, e) = fixture();
    let repo = b.join("repo");
    fs::create_dir_all(repo.join(".git/worktrees/feature")).unwrap();
    let feat = repo.join(".worktrees/feature");
    fs::create_dir_all(feat.join(".claude")).unwrap();
    fs::create_dir_all(feat.join("src")).unwrap();
    fs::write(
        feat.join(".git"),
        format!("gitdir: {}/.git/worktrees/feature\n", s(&repo)),
    )
    .unwrap();
    assert_eq!(find_project_root(&feat.join("src"), no_cfg(), &e), repo);
}
#[test]
fn pr_12_linked_worktree_roots() {
    let (_t, b, e) = fixture();
    let repo = b.join("repo");
    fs::create_dir_all(repo.join(".git/worktrees/linked-feature")).unwrap();
    let lf = b.join("linked-feature");
    fs::create_dir_all(lf.join("src")).unwrap();
    fs::write(
        lf.join(".git"),
        format!("gitdir: {}/.git/worktrees/linked-feature\n", s(&repo)),
    )
    .unwrap();
    assert_eq!(
        find_git_workspace_roots(&lf.join("src"), &e),
        [s(&lf), s(&repo)]
    );
}
#[test]
fn pr_13_relative_gitdir_nested_worktree_omitted() {
    let (_t, b, e) = fixture();
    let repo = b.join("repo");
    fs::create_dir_all(repo.join(".git/worktrees/full-local")).unwrap();
    let wt = repo.join(".worktrees/full-local");
    fs::create_dir_all(wt.join("notes")).unwrap();
    fs::write(wt.join(".git"), "gitdir: ../../.git/worktrees/full-local\n").unwrap();
    assert_eq!(find_git_workspace_roots(&wt.join("notes"), &e), [s(&repo)]);
}
#[test]
fn pr_14_stop_at_home_false() {
    let (_t, b, e) = fixture();
    let sub = b.join("fake-home/workspace/project");
    fs::create_dir_all(&sub).unwrap();
    fs::create_dir(b.join("fake-home/.git")).unwrap();
    let o = FindProjectRootOptions {
        create_config: false,
        stop_at_home: false,
    };
    assert_eq!(find_project_root(&sub, o, &e), b.join("fake-home"));
}
#[test]
fn pr_15_both_markers() {
    let (_t, b, e) = fixture();
    let sub = b.join("both-markers/src");
    fs::create_dir_all(&sub).unwrap();
    fs::create_dir(b.join("both-markers/.claude")).unwrap();
    fs::create_dir(b.join("both-markers/.git")).unwrap();
    assert_eq!(
        find_project_root(&sub, FindProjectRootOptions::default(), &e),
        b.join("both-markers")
    );
}
#[test]
fn pr_16_existing_config_untouched() {
    let (_t, b, e) = fixture();
    let d = b.join("existing-config");
    fs::create_dir_all(d.join(".config/safesh")).unwrap();
    let f = d.join(".config/safesh/config.local.json");
    fs::write(&f, "{\"existing\":\"data\"}\n").unwrap();
    assert_eq!(
        find_project_root(&d, FindProjectRootOptions::default(), &e),
        d
    );
    assert_eq!(fs::read_to_string(f).unwrap(), "{\"existing\":\"data\"}\n");
}
#[test]
fn pr_17_markers() {
    assert_eq!(PROJECT_MARKERS, [".claude", ".git", ".config/safesh"]);
}
#[test]
fn pr_18_tilde_expansion() {
    let e = env(Path::new("/Users/test-home"));
    assert_eq!(
        normalize_workspace_roots(&["~/dev".into()], &e),
        ["/Users/test-home/dev"]
    );
    assert_eq!(
        normalize_workspace_roots(&["~".into()], &e),
        ["/Users/test-home"]
    );
}
#[test]
fn pr_19_absolute_root() {
    let e = env(Path::new("/Users/test-home"));
    assert_eq!(
        normalize_workspace_roots(&["/abs/root".into()], &e),
        ["/abs/root"]
    );
}
#[test]
fn pr_20_nested_dropped() {
    let e = env(Path::new("/Users/test-home"));
    assert_eq!(
        normalize_workspace_roots(&["~/dev".into(), "~/dev/safesh".into()], &e),
        ["/Users/test-home/dev"]
    );
}
#[test]
fn pr_normalize_dedupes_and_expand_git_roots() {
    let (_t, b, e) = fixture();
    let e2 = env(Path::new("/h"));
    assert_eq!(
        normalize_workspace_roots(&["/a/b/".into(), "/a/b".into()], &e2),
        ["/a/b"]
    );
    let repo = b.join("repo");
    fs::create_dir_all(repo.join(".git/worktrees/w")).unwrap();
    let w = b.join("w");
    fs::create_dir_all(&w).unwrap();
    fs::write(
        w.join(".git"),
        format!("gitdir: {}/.git/worktrees/w\n", s(&repo)),
    )
    .unwrap();
    assert_eq!(expand_git_workspace_roots(&[s(&w)], &e), [s(&w), s(&repo)]);
}
