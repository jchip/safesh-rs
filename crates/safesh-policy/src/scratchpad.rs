//! Trust rules for Claude Code session scratchpad paths.
//!
//! Scratchpads live at `/private/tmp/claude-<uid>/<project>/<session-id>/scratchpad`. Scripts there are
//! trusted for that session only. The session id and uid are explicit parameters.

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

/// Lexical check: `path` sits below this session's scratchpad dir.
#[must_use]
pub fn is_under_session_scratchpad(path: &str, session_id: Option<&str>, uid: Option<u32>) -> bool {
    let (Some(sid), Some(uid)) = (session_id, uid) else {
        return false;
    };
    if !valid_session_id(sid) || !path.starts_with('/') {
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

fn owns_claude_dir(uid: u32) -> bool {
    fs_err::metadata(format!("/tmp/claude-{uid}")).is_ok_and(|m| m.uid() == uid)
}

/// Whether `path` is an existing file in this session's scratchpad. The real path must stay inside
/// it too, and `/tmp/claude-<uid>` must belong to us.
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
    match fs_err::symlink_metadata(path) {
        Ok(_) => return false,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return false,
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
