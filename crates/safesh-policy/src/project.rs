//! Project root and git workspace root detection.

use std::path::{Path, PathBuf};

use crate::host::HostEnv;
use crate::paths::{normalize_lexical, resolve_lexical};

/// Directories that mark a project root, in check order.
pub const PROJECT_MARKERS: [&str; 3] = [".claude", ".git", ".config/safesh"];

#[derive(Debug, Clone, Copy)]
pub struct FindProjectRootOptions {
    /// Create `.config/safesh/config.local.json` in cwd when no marker is found.
    pub create_config: bool,
    /// Stop the upward walk at HOME and never return it.
    pub stop_at_home: bool,
}

impl Default for FindProjectRootOptions {
    fn default() -> Self {
        Self {
            create_config: true,
            stop_at_home: true,
        }
    }
}

fn is_within_root(path: &str, root: &str) -> bool {
    if cfg!(windows) {
        return crate::winpath::is_within(path, root);
    }
    path == root || path.strip_prefix(root).is_some_and(|r| r.starts_with('/'))
}

/// Parent of an absolute path, `None` at the root.
fn parent_dir(dir: &str) -> Option<String> {
    let parent = Path::new(dir).parent()?;
    let p = parent.to_string_lossy().into_owned();
    (p != dir && !p.is_empty()).then_some(p)
}

/// Parse `gitdir: <path>` from a `.git` file. Single line only, as the TS regex requires.
fn parse_gitdir_file(content: &str) -> Option<String> {
    let rest = content.trim().strip_prefix("gitdir:")?;
    if rest.contains(['\n', '\r', '\u{2028}', '\u{2029}']) {
        return None;
    }
    let raw = rest.trim();
    (!raw.is_empty()).then(|| raw.to_string())
}

/// Git dir behind a `.git` marker: the marker itself for a directory, the `gitdir:` target for a file.
fn read_git_dir_path(worktree_root: &str, git_marker: &str) -> Option<String> {
    let meta = fs_err::metadata(git_marker).ok()?;
    if meta.is_dir() {
        return Some(git_marker.to_string());
    }
    if !meta.is_file() {
        return None;
    }
    let content = fs_err::read_to_string(git_marker).ok()?;
    let raw = parse_gitdir_file(&content)?;
    Some(if crate::paths::is_absolute_str(&raw) {
        raw
    } else {
        resolve_lexical(worktree_root, &raw)
    })
}

fn top_worktree_from_git_dir(git_dir: &str) -> Option<String> {
    if cfg!(windows) {
        // Git writes `gitdir:` with `/` on Windows. Search one spelling.
        let g = crate::winpath::normalize(git_dir);
        return g.rfind(r"\.git\worktrees\").map(|i| g[..i].to_string());
    }
    git_dir
        .rfind("/.git/worktrees/")
        .map(|i| git_dir[..i].to_string())
}

fn conventional_worktree_parent(dir: &str, git_file: &str) -> Option<String> {
    let git_dir = read_git_dir_path(dir, git_file)?;
    let parent = top_worktree_from_git_dir(&git_dir)?;
    dir.starts_with(&format!("{parent}/.worktrees/"))
        .then_some(parent)
}

/// Nearest checkout root at or above `cwd`, and its top worktree root if linked.
fn find_git_worktree_metadata(cwd: &str) -> Option<(String, Option<String>)> {
    let mut dir = resolve_lexical("/", cwd);
    loop {
        let marker = format!("{dir}/.git");
        if let Some(git_dir) = read_git_dir_path(&dir, &marker) {
            return Some((dir, top_worktree_from_git_dir(&git_dir)));
        }
        dir = parent_dir(&dir)?;
    }
}

fn expand_home_tilde(path: &str, env: &HostEnv) -> String {
    let home = env.home.to_string_lossy();
    if path == "~" {
        return home.into_owned();
    }
    match path.strip_prefix("~/") {
        Some(rest) if !home.is_empty() => format!("{home}/{rest}"),
        _ => path.to_string(),
    }
}

/// Expand `~`, normalize, dedupe, and drop roots nested inside another root.
/// Relative roots are normalized but not made absolute.
#[must_use]
pub fn normalize_workspace_roots(roots: &[String], env: &HostEnv) -> Vec<String> {
    let mut unique: Vec<String> = Vec::new();
    for r in roots {
        let mut n = normalize_lexical(&expand_home_tilde(r, env));
        while n.len() > 1 && n.ends_with('/') {
            n.pop();
        }
        if !unique.contains(&n) {
            unique.push(n);
        }
    }
    unique
        .iter()
        .enumerate()
        .filter(|(i, root)| {
            !unique
                .iter()
                .enumerate()
                .any(|(j, other)| *i != j && is_within_root(root, other))
        })
        .map(|(_, r)| r.clone())
        .collect()
}

/// Checkout root plus its top worktree root, for a git checkout containing `cwd`.
#[must_use]
pub fn find_git_workspace_roots(cwd: &Path, env: &HostEnv) -> Vec<String> {
    let Some((root, top)) = find_git_worktree_metadata(&cwd.to_string_lossy()) else {
        return Vec::new();
    };
    let mut roots = vec![root];
    roots.extend(top);
    normalize_workspace_roots(&roots, env)
}

/// Each root followed by its git workspace roots, normalized.
#[must_use]
pub fn expand_git_workspace_roots(roots: &[String], env: &HostEnv) -> Vec<String> {
    let mut out = Vec::new();
    for r in roots {
        out.push(r.clone());
        out.extend(find_git_workspace_roots(Path::new(r), env));
    }
    normalize_workspace_roots(&out, env)
}

fn create_fallback_config(cwd: &str) {
    let dir = format!("{cwd}/.config/safesh");
    if fs_err::create_dir_all(&dir).is_err() {
        return;
    }
    let file = format!("{dir}/config.local.json");
    if fs_err::metadata(&file).is_err() {
        let _ = fs_err::write(&file, "{}\n");
    }
}

/// Find the project root for `cwd`. The result is never canonicalized.
///
/// Order: `CLAUDE_PROJECT_DIR`, a conventional `.worktrees` checkout's parent project, the nearest
/// `.claude` dir, the nearest `.git` or `.config/safesh` dir, then `cwd` itself (creating a config
/// when `create_config` is set).
#[must_use]
pub fn find_project_root(cwd: &Path, opts: FindProjectRootOptions, env: &HostEnv) -> PathBuf {
    if let Some(p) = &env.claude_project_dir {
        return p.clone();
    }
    let cwd_s = cwd.to_string_lossy().into_owned();
    let home = env.home.to_string_lossy();
    let mut nearest: Option<String> = None;
    let mut dir = cwd_s.clone();
    loop {
        if opts.stop_at_home && !home.is_empty() && dir == home {
            break;
        }
        if let Some(p) = conventional_worktree_parent(&dir, &format!("{dir}/.git")) {
            return PathBuf::from(p);
        }
        for marker in PROJECT_MARKERS {
            if fs_err::metadata(format!("{dir}/{marker}")).is_ok() {
                if marker == ".claude" {
                    return PathBuf::from(dir);
                }
                nearest.get_or_insert_with(|| dir.clone());
            }
        }
        // Strip the last segment; stop at the filesystem root.
        let parent = match dir.rfind('/') {
            Some(i) => dir[..i].to_string(),
            None => dir.clone(),
        };
        if parent == dir || parent.is_empty() {
            break;
        }
        dir = parent;
    }
    if let Some(n) = nearest {
        return PathBuf::from(n);
    }
    if opts.create_config {
        create_fallback_config(&cwd_s);
    }
    cwd.to_path_buf()
}
