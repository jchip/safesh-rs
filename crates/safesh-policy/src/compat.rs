//! SPIKE(windows): platform shims so Unix-only file-mode calls compile on Windows.
//!
//! On Unix this re-exports `std::os::unix::fs`. On Windows `.mode()` is a no-op, so files and dirs
//! get the parent's ACL instead of 0600/0700. A real port needs an owner-only DACL here.

pub mod fs {
    #[cfg(unix)]
    pub use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};

    #[cfg(not(unix))]
    pub trait DirBuilderExt {
        fn mode(&mut self, mode: u32) -> &mut Self;
    }

    #[cfg(not(unix))]
    impl DirBuilderExt for std::fs::DirBuilder {
        fn mode(&mut self, _mode: u32) -> &mut Self {
            self
        }
    }

    #[cfg(not(unix))]
    pub trait OpenOptionsExt {
        fn mode(&mut self, mode: u32) -> &mut Self;
    }

    #[cfg(not(unix))]
    impl OpenOptionsExt for std::fs::OpenOptions {
        fn mode(&mut self, _mode: u32) -> &mut Self {
            self
        }
    }
}

/// Whether a process with this pid exists. EPERM counts as alive.
#[must_use]
pub fn pid_alive(pid: i32) -> bool {
    #[cfg(unix)]
    {
        let Some(pid) = rustix::process::Pid::from_raw(pid) else {
            return false;
        };
        !matches!(
            rustix::process::test_kill_process(pid),
            Err(rustix::io::Errno::SRCH)
        )
    }
    // SPIKE(windows): no liveness check; stale queue tickets are never reaped.
    #[cfg(not(unix))]
    {
        pid > 0
    }
}
