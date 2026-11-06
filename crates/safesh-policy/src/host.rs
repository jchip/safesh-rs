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
            home: var("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from),
            tmpdir: var("TMPDIR").map(PathBuf::from),
            xdg_config_home: var("XDG_CONFIG_HOME").map(PathBuf::from),
            path: var("PATH"),
            claude_project_dir: var("CLAUDE_PROJECT_DIR").map(PathBuf::from),
            uid: rustix::process::getuid().as_raw(),
            state_dir: var("SAFESH_STATE_DIR")
                .map_or_else(|| PathBuf::from("/tmp/safesh"), PathBuf::from),
        }
    }
}
