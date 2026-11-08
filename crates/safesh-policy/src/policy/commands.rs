//! Multi-command and multi-cwd helpers over [`Policy::check_run`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::Policy;
use crate::effect::{Decision, Reason};
use crate::paths::{has_path_sep, is_absolute_str, resolve_lexical};

/// Port of TS `candidateCwds`: the base cwd plus each statically known cwd a relative path command may run in.
/// `None` descriptors and `""` fall back to the base cwd. Bare names and absolute paths ignore descriptors.
#[must_use]
pub fn candidate_cwds(
    command: &str,
    base_cwd: &Path,
    descriptors: Option<&[Option<String>]>,
) -> Vec<PathBuf> {
    let mut cwds = vec![base_cwd.to_path_buf()];
    let Some(descriptors) = descriptors else {
        return cwds;
    };
    if is_absolute_str(command) || !has_path_sep(command) {
        return cwds;
    }
    let base = base_cwd.to_string_lossy();
    for d in descriptors {
        let cwd = d.as_deref().map_or_else(
            || base_cwd.to_path_buf(),
            |d| PathBuf::from(resolve_lexical(&base, d)),
        );
        if !cwds.contains(&cwd) {
            cwds.push(cwd);
        }
    }
    cwds
}

/// Port of TS `checkCommandAllowedInAnyCwd`: the first allowing decision, else the last decision.
/// With no cwds the result is `Deny` with `NotFound`.
#[must_use]
pub fn check_command_allowed_in_any_cwd(
    policy: &Policy,
    command: &str,
    cwds: &[PathBuf],
) -> Decision {
    let mut last = Decision::Deny {
        resolved: command.into(),
        reason: Reason::NotFound,
    };
    for cwd in cwds {
        let d = policy.check_run(command, cwd);
        if d.is_allow() {
            return d;
        }
        last = d;
    }
    last
}

/// Port of TS `MultiCommandResult`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiCommandResult {
    pub all_allowed: bool,
    /// Decision by command name.
    pub results: BTreeMap<String, Decision>,
    /// Command paths that need approval.
    pub not_allowed: Vec<String>,
    /// Command paths that do not exist.
    pub not_found: Vec<String>,
}

/// Port of TS `checkMultipleCommands`. `commands` pairs a name with the command path to check.
#[must_use]
pub fn check_multiple_commands(
    policy: &Policy,
    commands: &[(&str, &str)],
    cwd: &Path,
) -> MultiCommandResult {
    let mut out = MultiCommandResult {
        all_allowed: true,
        results: BTreeMap::new(),
        not_allowed: Vec::new(),
        not_found: Vec::new(),
    };
    for (name, path) in commands {
        let d = policy.check_run(path, cwd);
        match &d {
            Decision::Allow { .. } => {}
            Decision::Deny {
                reason: Reason::NotFound,
                ..
            } => out.not_found.push((*path).to_string()),
            _ => out.not_allowed.push((*path).to_string()),
        }
        out.results.insert((*name).to_string(), d);
    }
    out.all_allowed = out.not_allowed.is_empty() && out.not_found.is_empty();
    out
}
