//! Lexical path helpers. Nothing here touches the filesystem.

use std::path::{Component, Path, PathBuf};

/// Collapse `.` and `..` without resolving symlinks.
pub(crate) fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() && !p.is_absolute() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}

pub(crate) const DEVICE_PATHS: [&str; 5] = [
    "/dev/null",
    "/dev/stdout",
    "/dev/stderr",
    "/dev/stdin",
    "/dev/tty",
];

pub(crate) fn is_device(path: &str) -> bool {
    // Not `Path::is_absolute`: on Windows `/dev/null` has no drive, so it isn't absolute there.
    path.starts_with('/')
        && DEVICE_PATHS.contains(&normalize(Path::new(path)).to_string_lossy().as_ref())
}
