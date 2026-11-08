//! Path expansion, containment checks and default path lists.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::host::HostEnv;
use crate::winpath;

/// The command name in `prog`, for matching against names: the last path part. On Windows a
/// `PATHEXT` extension is dropped too, so `C:\Git\usr\bin\ls.exe` is `ls`.
#[must_use]
pub fn command_name(prog: &str) -> &str {
    if cfg!(windows) {
        winpath::command_name(prog, winpath::pathext())
    } else {
        prog.rsplit('/').next().unwrap_or(prog)
    }
}

/// Whether `s` is a path rather than a bare command name.
#[must_use]
pub fn has_path_sep(s: &str) -> bool {
    s.contains('/') || (cfg!(windows) && s.contains('\\'))
}

/// Whether `s` is an absolute path on this host. On Windows a Git Bash rooted path such as `/c/x`
/// counts, since [`resolve_lexical`] maps it.
#[must_use]
pub fn is_absolute_str(s: &str) -> bool {
    s.starts_with('/') || (cfg!(windows) && winpath::is_absolute(s))
}

/// The host path for a rooted spelling the OS would misread: Git Bash `/c/x` or `/tmp` on Windows.
/// `None` on other hosts and for every other path. The engine maps paths through this, so it opens
/// the same file the policy checked.
#[must_use]
pub fn host_path(path: &Path) -> Option<PathBuf> {
    if !cfg!(windows) {
        return None;
    }
    let s = path.to_str()?;
    // Devices keep their spelling: policy matches `/dev/null` by name, and the engine opens `NUL`.
    if s == "/dev" || s.starts_with("/dev/") {
        return Some(path.to_path_buf());
    }
    winpath::msys_to_windows(s, winpath::mounts()).map(PathBuf::from)
}

/// Whether two absolute paths name the same file as this host compares names.
#[must_use]
pub fn same_path(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        winpath::same_path(&a.to_string_lossy(), &b.to_string_lossy())
    } else {
        a == b
    }
}

/// A canonical path in the spelling policy compares: Windows' `\\?\` prefix is dropped.
#[must_use]
pub fn clean_canonical(p: PathBuf) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(winpath::normalize(&p.to_string_lossy()))
    } else {
        p
    }
}

/// Lexically normalize a POSIX path: collapse `//` and `.`, resolve `..`.
/// A trailing slash is kept, as `@std/path` `normalize` does. Relative paths stay relative.
#[must_use]
pub fn normalize_lexical(path: &str) -> String {
    if cfg!(windows) {
        return winpath::normalize(path);
    }
    if path.is_empty() {
        return ".".to_string();
    }
    let absolute = path.starts_with('/');
    let trailing = path.ends_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|l| *l != "..") {
                    parts.pop();
                } else if !absolute {
                    parts.push("..");
                }
            }
            s => parts.push(s),
        }
    }
    let mut out = parts.join("/");
    if absolute {
        out.insert(0, '/');
    } else if out.is_empty() {
        out.push('.');
    }
    if trailing && !out.ends_with('/') {
        out.push('/');
    }
    out
}

/// Like `@std/path` `resolve(cwd, path)`: absolute against `cwd`, normalized, no trailing slash.
#[must_use]
pub fn resolve_lexical(cwd: &str, path: &str) -> String {
    if cfg!(windows) {
        return winpath::resolve(cwd, path, winpath::mounts());
    }
    let joined = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("{cwd}/{path}")
    };
    let mut out = normalize_lexical(&joined);
    while out.len() > 1 && out.ends_with('/') {
        out.pop();
    }
    out
}

/// True when `path` equals `parent` or sits below it. Both should be absolute.
#[must_use]
pub fn is_path_within(path: &str, parent: &str) -> bool {
    if cfg!(windows) {
        return winpath::is_within(path, parent);
    }
    path == parent
        || path
            .strip_prefix(parent)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// True when `path` is within any of `allowed`.
#[must_use]
pub fn is_path_within_any<S: AsRef<str>>(path: &str, allowed: &[S]) -> bool {
    allowed.iter().any(|a| is_path_within(path, a.as_ref()))
}

/// Resolve `path` and each allowed entry against `cwd`, then test containment.
#[must_use]
pub fn is_path_within_allowed<S: AsRef<str>>(path: &str, allowed: &[S], cwd: &Path) -> bool {
    let cwd = cwd.to_string_lossy();
    let abs = resolve_lexical(&cwd, path);
    let abs_allowed: Vec<String> = allowed
        .iter()
        .map(|p| resolve_lexical(&cwd, p.as_ref()))
        .collect();
    is_path_within_any(&abs, &abs_allowed)
}

/// True when `path` (relative ones resolve against `cwd`) is within `project_dir`.
#[must_use]
pub fn is_within_project_dir(path: &str, project_dir: &str, cwd: &Path) -> bool {
    let cwd = cwd.to_string_lossy();
    let abs = resolve_lexical(&cwd, path);
    let proj = resolve_lexical(&cwd, project_dir);
    is_path_within(&abs, &proj)
}

/// Like [`is_within_project_dir`], but bare command names (no slash) are never within.
#[must_use]
pub fn is_command_within_project_dir(command_path: &str, project_dir: &str, cwd: &Path) -> bool {
    if !has_path_sep(command_path) {
        return false;
    }
    is_within_project_dir(command_path, project_dir, cwd)
}

/// Replace `$NAME` where the next char is not a word char (JS `\b` after a word char).
fn replace_bare_var(s: &str, name: &str, value: &str) -> String {
    let needle = format!("${name}");
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find(&needle) {
        let after = &rest[i + needle.len()..];
        let boundary = after
            .chars()
            .next()
            .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_'));
        out.push_str(&rest[..i]);
        if boundary {
            out.push_str(value);
        } else {
            out.push_str(&needle);
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

/// Expand `~`, `${CWD}`, `${HOME}`, `${WORKSPACE}` and the bare `$` forms. No normalization.
#[must_use]
pub fn expand_path(path: &str, cwd: &Path, workspace: Option<&str>, env: &HostEnv) -> String {
    let home = env.home.to_string_lossy();
    let cwd = cwd.to_string_lossy();
    let ws = workspace.unwrap_or("");
    let expanded = if path == "~" {
        home.to_string()
    } else if let Some(rest) = path.strip_prefix("~/") {
        format!("{home}/{rest}")
    } else {
        path.to_string()
    };
    let s = expanded
        .replace("${CWD}", &cwd)
        .replace("${HOME}", &home)
        .replace("${WORKSPACE}", ws);
    let s = replace_bare_var(&s, "CWD", &cwd);
    let s = replace_bare_var(&s, "HOME", &home);
    replace_bare_var(&s, "WORKSPACE", ws)
}

/// True when `path` is within any of `allowed` after expansion and resolution against `cwd`.
#[must_use]
pub fn is_path_allowed<S: AsRef<str>>(
    path: &str,
    allowed: &[S],
    cwd: &Path,
    workspace: Option<&str>,
    env: &HostEnv,
) -> bool {
    let cwd_s = cwd.to_string_lossy();
    let abs = resolve_lexical(&cwd_s, path);
    allowed.iter().any(|a| {
        let exp = expand_path(a.as_ref(), cwd, workspace, env);
        is_path_within(&abs, &resolve_lexical(&cwd_s, &exp))
    })
}

/// Temp-dir entries for the default read/write lists: `/tmp`, the OS temp dir, and their real paths.
#[must_use]
pub fn compute_temp_dir_defaults(tmpdir: Option<&str>) -> Vec<String> {
    let mut dirs = Vec::new();
    let mut push = |dir: &str| {
        dirs.push(dir.to_string());
        if let Ok(real) = fs_err::canonicalize(dir) {
            let real = clean_canonical(real).to_string_lossy().into_owned();
            if real != dir {
                dirs.push(real);
            }
        }
    };
    push("/tmp");
    if let Some(t) = tmpdir {
        let os_tmp = t.trim_end_matches('/');
        if !os_tmp.is_empty() && os_tmp != "/tmp" && os_tmp != "/private/tmp" {
            push(os_tmp);
        }
    }
    dirs
}

/// Absolute `$PATH` entries, trailing separators stripped, deduped, order kept.
#[must_use]
pub fn compute_path_dir_defaults(path_env: Option<&OsStr>) -> Vec<String> {
    let Some(p) = path_env else {
        return Vec::new();
    };
    let p = p.to_string_lossy();
    let mut dirs: Vec<String> = Vec::new();
    let sep = if cfg!(windows) { ';' } else { ':' };
    for raw in p.split(sep) {
        let dir = raw.trim_end_matches(['/', '\\']);
        let dir = if cfg!(windows) {
            winpath::normalize(dir)
        } else {
            dir.to_string()
        };
        let absolute = if cfg!(windows) {
            winpath::is_absolute(&dir)
        } else {
            dir.starts_with('/')
        };
        let seen = dirs.iter().any(|d| {
            if cfg!(windows) {
                d.eq_ignore_ascii_case(&dir)
            } else {
                *d == dir
            }
        });
        if dir.is_empty() || !absolute || seen {
            continue;
        }
        dirs.push(dir);
    }
    dirs
}

/// Real directories named `.*` directly under an absolute `home`, sorted. Symlinks are skipped.
#[must_use]
pub fn compute_home_dot_dir_defaults(home: Option<&Path>) -> Vec<String> {
    let Some(home) = home.filter(|h| h.is_absolute()) else {
        return Vec::new();
    };
    let Ok(rd) = fs_err::read_dir(home) else {
        return Vec::new();
    };
    let mut dirs = Vec::new();
    for entry in rd {
        let Ok(entry) = entry else {
            return Vec::new();
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') && entry.file_type().is_ok_and(|t| t.is_dir()) {
            dirs.push(home.join(&name).to_string_lossy().into_owned());
        }
    }
    dirs.sort();
    dirs
}

/// Fold the macOS `/private/tmp` spelling onto `/tmp`.
fn fold_private_tmp(path: PathBuf) -> PathBuf {
    match path.strip_prefix("/private/tmp") {
        Ok(rest) => Path::new("/tmp").join(rest),
        Err(_) => path,
    }
}

/// Absolute, `..`-normalized path with symlinks resolved for the longest existing prefix.
/// The missing tail is appended as written. `/private/tmp` folds onto `/tmp`.
/// Policy calls this before every path decision.
#[must_use]
pub fn resolve_path(path: &Path, cwd: &Path) -> PathBuf {
    let lexical = resolve_lexical(&cwd.to_string_lossy(), &path.to_string_lossy());
    let real = soft_canonicalize::soft_canonicalize(&lexical)
        .map_or_else(|_| lexical.into(), clean_canonical);
    fold_private_tmp(real)
}
