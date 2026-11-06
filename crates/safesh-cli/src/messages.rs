//! User-facing denial text for the runtime gate.

use std::fmt::Write as _;

/// A command that needs approval, with the retry menu.
pub fn blocked(commands: &[String], pending_id: &str, reasons: &[String]) -> String {
    let mut msg = safesh_hooks::blocked_message(commands, pending_id);
    for r in reasons {
        let _ = write!(msg, "\nNOTE: {r}");
    }
    msg
}

/// A path that needs approval, with the retry-path menu.
pub fn path_blocked(path: &str, write: bool, is_dir: bool, pending_id: &str) -> String {
    safesh_hooks::path_blocked_message(path, write, is_dir, pending_id)
}

/// A denial approval can't fix, or one with nothing to retry.
pub fn denied(what: &str, reasons: &[String]) -> String {
    if reasons.is_empty() {
        format!("[SAFESH] DENIED: {what}")
    } else {
        format!("[SAFESH] DENIED: {what} ({})", reasons.join("; "))
    }
}
