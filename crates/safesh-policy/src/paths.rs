//! Path expansion, containment checks and default path lists.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::host::HostEnv;

/// Lexically normalize a POSIX path: collapse `//` and `.`, resolve `..`.
/// A trailing slash is kept, as `@std/path` `normalize` does. Relative paths stay relative.
#[must_use]
pub fn normalize_lexical(path: &str) -> String {
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
    if !command_path.contains('/') && !command_path.contains('\\') {
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
            let real = real.to_string_lossy().into_owned();
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
    for raw in p.split(':') {
        let dir = raw.trim_end_matches(['/', '\\']);
        if dir.is_empty() || !dir.starts_with('/') || dirs.iter().any(|d| d == dir) {
            continue;
        }
        dirs.push(dir.to_string());
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
    let real = soft_canonicalize::soft_canonicalize(&lexical).unwrap_or_else(|_| lexical.into());
    fold_private_tmp(real)
}
