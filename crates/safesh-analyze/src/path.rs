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
    if !path.starts_with('/') {
        return false;
    }
    let norm = normalize(Path::new(path)).to_string_lossy().into_owned();
    // Windows `normalize` joins with `\`, so compare in `/` spelling there. On Unix `\` is an
    // ordinary file-name char: `/dev\null` is a file in `/`, not a device.
    let norm = if cfg!(windows) {
        norm.replace('\\', "/")
    } else {
        norm
    };
    DEVICE_PATHS.contains(&norm.as_str())
}

#[cfg(test)]
mod device_tests {
    use super::is_device;

    #[test]
    fn devices_by_name() {
        assert!(is_device("/dev/null"));
        assert!(is_device("/dev/./null"));
        assert!(!is_device("/dev/sda"));
        assert!(!is_device("dev/null"));
    }

    #[test]
    #[cfg(unix)]
    fn backslash_is_a_name_char_on_unix() {
        assert!(!is_device("/dev\\null"));
    }
}
