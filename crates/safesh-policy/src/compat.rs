//! Platform shims: Unix file modes and process calls, with their Windows equivalents.
//!
//! The Windows half is the workspace's only `unsafe` code (Win32 FFI). It lives in `win` below,
//! and this crate lowers `unsafe_code` from `forbid` to `deny` so that one module can opt in.

use std::io;
use std::path::Path;

/// `.mode()` on `DirBuilder` and `OpenOptions`. On Unix this re-exports `std::os::unix::fs`.
/// On Windows `.mode()` is a no-op: a new file or dir takes the DACL its parent passes down, so
/// call [`restrict_dir`] on the dir that holds private files.
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

/// Makes `dir` private to the current user.
///
/// Unix: mode 0700. Windows: a protected DACL, so nothing is inherited from above, with one
/// full-control ACE for the current user that files and subdirs inherit. Files created inside
/// need no call of their own, and files already there get the new inherited ACE.
pub fn restrict_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
    }
    #[cfg(windows)]
    {
        win::restrict_dir(dir)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = dir;
        Ok(())
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

/// Terminates process `pid` with exit status `code`, then waits up to `wait_ms` for it to be gone.
/// `TerminateProcess` returns before the process has exited, hence the wait. Errors are ignored:
/// a pid that no longer exists, or that we may not open, is skipped.
#[cfg(windows)]
pub fn terminate_and_wait(pid: u32, code: u32, wait_ms: u32) {
    win::terminate_and_wait(pid, code, wait_ms);
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod win {
    use std::io;
    use std::os::windows::ffi::OsStrExt as _;
    use std::path::Path;
    use std::ptr::{null, null_mut};

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::Authorization::{SE_FILE_OBJECT, SetNamedSecurityInfoW};
    use windows_sys::Win32::Security::{
        ACCESS_ALLOWED_ACE, ACL, ACL_REVISION, AddAccessAllowedAceEx, CONTAINER_INHERIT_ACE,
        DACL_SECURITY_INFORMATION, GetLengthSid, GetTokenInformation, InitializeAcl,
        OBJECT_INHERIT_ACE, PROTECTED_DACL_SECURITY_INFORMATION, TOKEN_QUERY, TOKEN_USER,
        TokenUser,
    };
    use windows_sys::Win32::Storage::FileSystem::FILE_ALL_ACCESS;
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
        TerminateProcess, WaitForSingleObject,
    };

    /// Closes the handle on drop.
    struct Handle(HANDLE);

    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: the handle came from a successful open and is closed only here.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    pub(super) fn terminate_and_wait(pid: u32, code: u32, wait_ms: u32) {
        // SAFETY: plain Win32 calls with no pointers; the handle is checked for null.
        unsafe {
            let h = OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, 0, pid);
            if h.is_null() {
                return;
            }
            let h = Handle(h);
            if TerminateProcess(h.0, code) != 0 {
                WaitForSingleObject(h.0, wait_ms);
            }
        }
    }

    /// This process's `TOKEN_USER`, in a buffer aligned for it. Its SID points into the same buffer.
    fn token_user() -> io::Result<Vec<u64>> {
        // SAFETY: the token handle is checked and closed by `Handle`. The first
        // `GetTokenInformation` call only reports the size; the second writes at most `len` bytes
        // into a buffer of at least `len` bytes, 8-byte aligned, which covers `TOKEN_USER`.
        unsafe {
            let mut token: HANDLE = null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token) == 0 {
                return Err(io::Error::last_os_error());
            }
            let token = Handle(token);
            let mut len = 0u32;
            GetTokenInformation(token.0, TokenUser, null_mut(), 0, &raw mut len);
            if len == 0 {
                return Err(io::Error::last_os_error());
            }
            let mut buf = vec![0u64; (len as usize).div_ceil(8)];
            if GetTokenInformation(token.0, TokenUser, buf.as_mut_ptr().cast(), len, &raw mut len)
                == 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(buf)
        }
    }

    pub(super) fn restrict_dir(dir: &Path) -> io::Result<()> {
        let user = token_user()?;
        let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: `user` holds a `TOKEN_USER` whose SID lives in `user`, which outlives every use
        // here. The ACL buffer is DWORD-aligned and sized for the header plus one ACE with that
        // SID, the size `InitializeAcl` and `AddAccessAllowedAceEx` require. `wide` is
        // NUL-terminated.
        unsafe {
            let sid = (*user.as_ptr().cast::<TOKEN_USER>()).User.Sid;
            let size = size_of::<ACL>() + size_of::<ACCESS_ALLOWED_ACE>() - size_of::<u32>()
                + GetLengthSid(sid) as usize;
            let mut acl = vec![0u32; size.div_ceil(4)];
            let acl_len = u32::try_from(acl.len() * 4).map_err(io::Error::other)?;
            let acl = acl.as_mut_ptr().cast::<ACL>();
            if InitializeAcl(acl, acl_len, ACL_REVISION) == 0
                || AddAccessAllowedAceEx(
                    acl,
                    ACL_REVISION,
                    OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE,
                    FILE_ALL_ACCESS,
                    sid,
                ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            let rc = SetNamedSecurityInfoW(
                wide.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                acl,
                null(),
            );
            if rc != 0 {
                return Err(io::Error::from_raw_os_error(
                    i32::try_from(rc).unwrap_or(i32::MAX),
                ));
            }
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        /// `icacls` output for `path`, one ACE per line, the path prefix stripped.
        fn aces(path: &std::path::Path) -> Vec<String> {
            let out = std::process::Command::new("icacls").arg(path).output().unwrap();
            let text = String::from_utf8_lossy(&out.stdout).replace(&*path.to_string_lossy(), "");
            text.lines()
                .map(str::trim)
                .take_while(|l| !l.is_empty())
                .map(str::to_string)
                .collect()
        }

        #[test]
        fn restrict_dir_leaves_one_owner_ace_that_files_inherit() {
            let t = tempfile::tempdir().unwrap();
            let d = t.path().join("private");
            std::fs::create_dir(&d).unwrap();
            let before = d.join("before.json");
            std::fs::write(&before, "{}").unwrap();
            super::super::restrict_dir(&d).unwrap();
            let after = d.join("after.json");
            std::fs::write(&after, "{}").unwrap();

            let user = std::env::var("USERNAME").unwrap().to_lowercase();
            let dir_aces = aces(&d);
            assert_eq!(dir_aces.len(), 1, "{dir_aces:?}");
            let ace = dir_aces[0].to_lowercase();
            assert!(ace.contains(&user), "{ace}");
            assert!(ace.contains("(oi)(ci)(f)"), "{ace}");
            assert!(!ace.contains("(i)"), "dir ACE must not be inherited: {ace}");
            for f in [&before, &after] {
                let file_aces = aces(f);
                assert_eq!(file_aces.len(), 1, "{}: {file_aces:?}", f.display());
                let ace = file_aces[0].to_lowercase();
                assert!(ace.contains(&user) && ace.contains("(i)(f)"), "{ace}");
            }
        }
    }
}
