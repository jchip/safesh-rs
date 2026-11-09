//! Snapshot of the host environment that policy code reads.
//!
//! Captured once per process with [`HostEnv::from_process`]. Tests build one directly.

use std::ffi::OsString;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostEnv {
    /// `$HOME`.
    pub home: PathBuf,
    /// `$TMPDIR`, if set.
    pub tmpdir: Option<PathBuf>,
    /// `$XDG_CONFIG_HOME`, if set.
    pub xdg_config_home: Option<PathBuf>,
    /// `$PATH`, if set.
    pub path: Option<OsString>,
    /// `$CLAUDE_PROJECT_DIR`, if set. Overrides project root detection.
    pub claude_project_dir: Option<PathBuf>,
    /// Real uid of this process.
    pub uid: u32,
    /// Root for safesh state: pending files, script cache. `$SAFESH_STATE_DIR`, else `/tmp/safesh`.
    pub state_dir: PathBuf,
}

impl HostEnv {
    #[must_use]
    pub fn from_process() -> Self {
        let var = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty());
        Self {
            home: home(&var).unwrap_or_else(|| PathBuf::from("/")),
            tmpdir: var("TMPDIR")
                .or_else(|| if cfg!(windows) { var("TEMP") } else { None })
                .map(PathBuf::from),
            xdg_config_home: var("XDG_CONFIG_HOME").map(PathBuf::from),
            path: var("PATH"),
            claude_project_dir: var("CLAUDE_PROJECT_DIR").map(PathBuf::from),
            #[cfg(unix)]
            uid: rustix::process::getuid().as_raw(),
            // Windows has no uid. Scratchpad trust there keys on `TEMP` and the dir owner instead.
            #[cfg(not(unix))]
            uid: 0,
            state_dir: var("SAFESH_STATE_DIR").map_or_else(default_state_dir, PathBuf::from),
        }
    }
}

/// `$HOME`. On Windows a Git Bash spelling (`/c/Users/me`) is mapped, and `USERPROFILE` is the
/// fallback, since a native process may not get `HOME` at all.
fn home(var: &impl Fn(&str) -> Option<std::ffi::OsString>) -> Option<PathBuf> {
    let home = var("HOME").map(PathBuf::from);
    if !cfg!(windows) {
        return home;
    }
    home.and_then(|h| match crate::paths::host_path(&h) {
        Some(mapped) => Some(mapped),
        None => crate::winpath::is_absolute(&h.to_string_lossy()).then_some(h),
    })
    .or_else(|| var("USERPROFILE").map(PathBuf::from))
}

/// `/tmp/safesh`. On Windows that would be `C:\tmp\safesh`, so the user temp dir is used instead.
fn default_state_dir() -> PathBuf {
    if cfg!(windows) {
        std::env::temp_dir().join("safesh")
    } else {
        PathBuf::from("/tmp/safesh")
    }
}
