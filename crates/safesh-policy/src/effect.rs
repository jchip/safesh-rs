//! Values that flow through the policy core: effects in, decisions out.

use std::ffi::OsString;
use std::path::PathBuf;

/// Read or write access to a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Access {
    Read,
    Write,
}

/// Something a script is about to do. Emitted by the exec hook, the open hook and the static pre-check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Exec {
        /// Program as written or as resolved by the shell.
        program: PathBuf,
        argv: Vec<OsString>,
        cwd: PathBuf,
    },
    Open {
        path: PathBuf,
        cwd: PathBuf,
        access: Access,
    },
}

/// One typed permission question, produced by interpreting an [`Effect`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Run a program. Relative paths are resolved against `cwd`; bare names through `PATH`.
    Run { program: PathBuf, cwd: PathBuf },
    /// Access a path. Relative paths are resolved against `cwd`.
    Path {
        path: PathBuf,
        cwd: PathBuf,
        access: Access,
    },
}

/// Why a request was not allowed outright.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// Not on any allow list. The user can approve it.
    NotAllowed,
    /// Matched a deny list.
    Denied,
    /// The program does not exist. Approval can't help.
    NotFound,
    /// Write to the project dir while `blockProjectDirWrite` is set.
    ProjectDirWriteBlocked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Allowed. Carries the resolved path the decision was made on.
    Allow {
        resolved: PathBuf,
    },
    Ask {
        resolved: PathBuf,
        reason: Reason,
    },
    Deny {
        resolved: PathBuf,
        reason: Reason,
    },
}

impl Decision {
    #[must_use]
    pub fn is_allow(&self) -> bool {
        matches!(self, Self::Allow { .. })
    }
}
