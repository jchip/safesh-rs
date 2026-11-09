//! Trust rules for Claude Code session scratchpad paths.
//!
//! Scratchpads live at `<root>/<project>/<session-id>/scratchpad`. Scripts there are trusted for
//! that session only. The root is Claude Code's per-user temp dir:
//!
//! - Unix: `/tmp/claude-<uid>` (`/private/tmp/...` on macOS). The uid is an explicit parameter.
//! - Windows: `<TEMP>\claude`, with `TEMP` resolved to its real path, the way Claude Code builds it
//!   (`realpath(os.tmpdir())` plus `claude`; Node has no uid there). `<project>` is the cwd with
//!   every non-alphanumeric char turned into `-`, so `C:\Users\me\proj` is `C--Users-me-proj`.
//!   Names compare without case, and either separator works.
//!
//! Claude Code also honors `CLAUDE_CODE_TMPDIR`. safesh does not, on either platform, so a
//! session that sets it gets no scratchpad trust.

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use crate::paths::normalize_lexical;

/// Session ids are UUID-like; anything else could smuggle path syntax.
fn valid_session_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Fold the macOS `/private/tmp` spelling onto `/tmp`.
fn fold_tmp(path: &str) -> &str {
    path.strip_prefix("/private")
        .filter(|_| path.starts_with("/private/tmp/"))
        .unwrap_or(path)
}

/// Lexical check: `path` sits below this session's scratchpad dir. On Windows `uid` only has to be
/// present; the root comes from `TEMP`.
#[must_use]
pub fn is_under_session_scratchpad(path: &str, session_id: Option<&str>, uid: Option<u32>) -> bool {
    let (Some(sid), Some(uid)) = (session_id, uid) else {
        return false;
    };
    if !valid_session_id(sid) {
        return false;
    }
    if cfg!(windows) {
        return windows_claude_root().is_some_and(|root| is_under_windows_root(path, sid, root));
    }
    if !path.starts_with('/') {
        return false;
    }
    let normalized = normalize_lexical(path);
    let folded = fold_tmp(&normalized);
    let Some(rest) = folded.strip_prefix(&format!("/tmp/claude-{uid}/")) else {
        return false;
    };
    // `<project>/<sid>/scratchpad/<non-slash>`
    let Some((project, rest)) = rest.split_once('/') else {
        return false;
    };
    if project.is_empty() {
        return false;
    }
    rest.strip_prefix(&format!("{sid}/scratchpad/"))
        .is_some_and(|tail| tail.chars().next().is_some_and(|c| c != '/'))
}

/// Windows form of the lexical check, against an explicit `root`. Pure string code, so it is tested
/// on every platform. `session_id` must already be valid.
#[must_use]
pub fn is_under_windows_root(path: &str, session_id: &str, root: &str) -> bool {
    use crate::winpath;
    if !winpath::is_absolute(path) || !winpath::is_absolute(root) {
        return false;
    }
    let path = winpath::normalize(path);
    let root = winpath::normalize(root);
    let root = root.trim_end_matches('\\');
    if !winpath::is_within(&path, root) || path.len() <= root.len() {
        return false;
    }
    // `is_within` matched `root` ignoring case, and the next char is `\`.
    let rest = &path[root.len() + 1..];
    let mut parts = rest.splitn(4, '\\');
    let (Some(project), Some(sid), Some(pad), Some(tail)) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    !project.is_empty()
        && sid.eq_ignore_ascii_case(session_id)
        && pad.eq_ignore_ascii_case("scratchpad")
        && !tail.is_empty()
        && !tail.starts_with('\\')
}

/// `<TEMP>\claude`, read once. `TEMP`, then `TMP`, as Node's `os.tmpdir()` reads them; a Git Bash
/// spelling is mapped. `None` when neither is an absolute path.
fn windows_claude_root() -> Option<&'static str> {
    static ROOT: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    ROOT.get_or_init(|| {
        let tmp = ["TEMP", "TMP"]
            .iter()
            .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))?;
        let tmp = crate::paths::host_path(Path::new(&tmp))
            .map_or(tmp, |p| p.to_string_lossy().into_owned());
        if !crate::winpath::is_absolute(&tmp) {
            return None;
        }
        let real = fs_err::canonicalize(&tmp).map_or(tmp, |p| p.to_string_lossy().into_owned());
        Some(format!(
            "{}\\claude",
            crate::winpath::normalize(&real).trim_end_matches('\\')
        ))
    })
    .as_deref()
}

#[cfg(unix)]
fn owns_claude_dir(uid: u32) -> bool {
    fs_err::metadata(format!("/tmp/claude-{uid}")).is_ok_and(|m| m.uid() == uid)
}

/// The root must be a real dir owned by us, or by Administrators or SYSTEM, which an elevated
/// Claude Code session leaves as owner.
#[cfg(windows)]
fn owns_claude_dir(_uid: u32) -> bool {
    windows_claude_root().is_some_and(|r| crate::compat::dir_owned_by_user_or_admins(Path::new(r)))
}

#[cfg(not(any(unix, windows)))]
fn owns_claude_dir(_uid: u32) -> bool {
    false
}

/// Whether `path` is an existing file in this session's scratchpad. The real path must stay inside
/// it too, and the root must belong to us.
#[must_use]
pub fn is_session_scratchpad_script(
    path: &str,
    session_id: Option<&str>,
    uid: Option<u32>,
) -> bool {
    if !is_under_session_scratchpad(path, session_id, uid) {
        return false;
    }
    let Some(uid) = uid else { return false };
    let Ok(real) = fs_err::canonicalize(path) else {
        return false;
    };
    let real_s = real.to_string_lossy();
    is_under_session_scratchpad(&real_s, session_id, Some(uid))
        && fs_err::metadata(&real).is_ok_and(|m| m.is_file())
        && owns_claude_dir(uid)
}

/// Whether `path` is a not-yet-written file in this session's scratchpad. The nearest existing parent
/// must resolve inside the scratchpad, so a symlinked dir cannot lead out.
#[must_use]
pub fn is_new_session_scratchpad_script(
    path: &str,
    session_id: Option<&str>,
    uid: Option<u32>,
) -> bool {
    if !is_under_session_scratchpad(path, session_id, uid) {
        return false;
    }
    let Some(uid) = uid else { return false };
    // Only a path that does not exist yet counts as a new script.
    if !matches!(fs_err::symlink_metadata(path), Err(e) if e.kind() == std::io::ErrorKind::NotFound)
    {
        return false;
    }
    let mut dir = normalize_lexical(path);
    loop {
        let Some(parent) = Path::new(&dir).parent() else {
            return false;
        };
        dir = parent.to_string_lossy().into_owned();
        match fs_err::canonicalize(&dir) {
            Ok(real) => {
                let probe = format!("{}/x", real.to_string_lossy());
                if !is_under_session_scratchpad(&probe, session_id, Some(uid)) {
                    return false;
                }
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return false,
        }
    }
    owns_claude_dir(uid)
}

#[cfg(test)]
mod tests {
    use super::is_under_windows_root;

    const SID: &str = "0b9d7a52-6f1e-4c3a-9d2b-1a2b3c4d5e6f";
    const ROOT: &str = r"C:\Users\me\AppData\Local\Temp\claude";

    fn pad() -> String {
        format!(r"{ROOT}\C--Users-me-proj\{SID}\scratchpad")
    }

    #[test]
    fn windows_under_pad() {
        assert!(is_under_windows_root(
            &format!(r"{}\run.sh", pad()),
            SID,
            ROOT
        ));
        assert!(is_under_windows_root(
            &format!(r"{}\a\b.sh", pad()),
            SID,
            ROOT
        ));
    }

    #[test]
    fn windows_case_separators_and_verbatim_prefix() {
        let p = format!(r"{}/Run.sh", pad())
            .to_uppercase()
            .replace('\\', "/");
        assert!(is_under_windows_root(&p, SID, ROOT));
        assert!(is_under_windows_root(
            &format!(r"\\?\{}\run.sh", pad()),
            SID,
            ROOT
        ));
        assert!(is_under_windows_root(
            &format!(r"{}\run.sh", pad()),
            SID,
            &format!(r"{ROOT}\")
        ));
    }

    #[test]
    fn windows_rejects() {
        let reject = |p: &str| assert!(!is_under_windows_root(p, SID, ROOT), "{p}");
        // Another session, the pad dir itself, traversal out, a relative path.
        reject(&format!(r"{ROOT}\C--Users-me-proj\other\scratchpad\run.sh"));
        reject(&format!(r"{}\", pad()));
        reject(&pad());
        reject(&format!(r"{}\..\..\x\run.sh", pad()));
        reject(r"claude\C--p\0b9d7a52-6f1e-4c3a-9d2b-1a2b3c4d5e6f\scratchpad\run.sh");
        // Missing project, wrong root, a sibling dir that only shares a prefix with the root.
        reject(&format!(r"{ROOT}\{SID}\scratchpad\run.sh"));
        reject(&format!(r"C:\Temp\claude\C--p\{SID}\scratchpad\run.sh"));
        reject(&format!(r"{ROOT}x\C--p\{SID}\scratchpad\run.sh"));
        // Not under `scratchpad`.
        reject(&format!(r"{ROOT}\C--p\{SID}\tasks\run.sh"));
    }
}
