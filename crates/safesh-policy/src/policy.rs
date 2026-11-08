//! Policy facade over `deno_permissions`. The only module that imports it.
//!
//! Path decisions follow TS `validatePath`: user deny wins, then `blockProjectDirWrite`, then allow lists.
//! Run decisions follow TS `checkCommandPermission`, except a path command must resolve to an allowed
//! program (D2) instead of matching by basename.

mod commands;
mod exec;
mod load;
mod sys;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use deno_permissions::{
    PermissionState, Permissions, PermissionsContainer, PermissionsOptions,
    RuntimePermissionDescriptorParser,
};

use crate::config::Config;
use crate::effect::{Access, Decision, Reason, Request};
use crate::host::HostEnv;
use crate::paths::{
    compute_home_dot_dir_defaults, compute_path_dir_defaults, compute_temp_dir_defaults,
    expand_path, is_path_within, is_path_within_any, resolve_lexical, resolve_path,
};
use crate::project::normalize_workspace_roots;
use sys::HostSys;

pub use commands::{
    MultiCommandResult, candidate_cwds, check_command_allowed_in_any_cwd, check_multiple_commands,
};
pub use exec::Problem;
pub use load::{LoadedSession, load_session_config, merge_session_permissions};

#[derive(Debug, thiserror::Error)]
pub enum PolicyError {
    #[error("invalid permission list: {0}")]
    Lists(#[from] deno_permissions::PermissionsFromOptionsError),
}

/// Port of TS `getEffectivePermissions`: config lists plus cwd, temp, HOME, PATH and root defaults.
/// Entries are returned as TS returns them: unexpanded config strings mixed with absolute defaults.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EffectivePermissions {
    pub read: Vec<String>,
    pub deny_read: Vec<String>,
    pub write: Vec<String>,
    pub deny_write: Vec<String>,
    pub run: Vec<String>,
}

/// `projectDir` plus `workspaceRoots`, normalized. Port of TS `getWorkspaceRoots`.
#[must_use]
pub fn workspace_roots(config: &Config, host: &HostEnv) -> Vec<String> {
    let mut roots: Vec<String> = config.project_dir.iter().cloned().collect();
    roots.extend(config.workspace_roots.iter().flatten().cloned());
    normalize_workspace_roots(&roots, host)
}

/// Value of `${WORKSPACE}`: `workspace`, else `workspaceDir`, with `~/` expanded and made absolute.
#[must_use]
pub fn workspace_variable(config: &Config, cwd: &Path, host: &HostEnv) -> Option<String> {
    let raw = config
        .workspace
        .as_ref()
        .or(config.workspace_dir.as_ref())?;
    Some(resolve_workspace(raw, cwd, host))
}

fn resolve_workspace(raw: &str, cwd: &Path, host: &HostEnv) -> String {
    let expanded = match raw.strip_prefix("~/") {
        Some(rest) => format!("{}/{rest}", host.home.to_string_lossy()),
        None => raw.to_string(),
    };
    resolve_lexical(&cwd.to_string_lossy(), &expanded)
}

#[must_use]
pub fn effective_permissions(config: &Config, cwd: &Path, host: &HostEnv) -> EffectivePermissions {
    let perms = config.permissions.clone().unwrap_or_default();
    let temp = compute_temp_dir_defaults(host.tmpdir.as_deref().and_then(Path::to_str));

    let mut read = vec![cwd.to_string_lossy().into_owned()];
    read.extend(temp.iter().cloned());
    if config.include_home_in_default_read != Some(false) {
        read.push(host.home.to_string_lossy().into_owned());
    }
    if config.include_path_dirs_in_default_read != Some(false) {
        read.extend(compute_path_dir_defaults(host.path.as_deref()));
    }
    let mut write = temp;
    if let Some(dir) = &config.workspace_dir {
        let dir = resolve_workspace(dir, cwd, host);
        read.push(dir.clone());
        write.push(dir);
    }

    let mut deny_write = perms.deny_write.clone().unwrap_or_default();
    let roots = workspace_roots(config, host);
    if !roots.is_empty() {
        read.extend(roots.iter().cloned());
        if config.block_project_dir_write == Some(true) {
            // A root holding an explicit write path stays out of the deny, as in TS.
            let workspace = workspace_variable(config, cwd, host);
            let explicit: Vec<String> = perms
                .write
                .iter()
                .flatten()
                .map(|p| expand_path(p, cwd, workspace.as_deref(), host))
                .collect();
            let cwd_s = cwd.to_string_lossy();
            deny_write.extend(
                roots
                    .iter()
                    .filter(|root| {
                        let root = resolve_lexical(&cwd_s, root);
                        !explicit
                            .iter()
                            .any(|p| is_path_within(&resolve_lexical(&cwd_s, p), &root))
                    })
                    .cloned(),
            );
        } else {
            write.extend(roots.iter().cloned());
        }
    }

    read.extend(perms.read.iter().flatten().cloned());
    write.extend(perms.write.iter().flatten().cloned());
    EffectivePermissions {
        read: dedupe(read),
        deny_read: perms.deny_read.unwrap_or_default(),
        write: dedupe(write),
        deny_write,
        run: perms.run.unwrap_or_default(),
    }
}

fn dedupe(v: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    v.into_iter().filter(|s| seen.insert(s.clone())).collect()
}

/// File tools TS ran as sandboxed builtins, so they never needed run permission. They stay
/// allowed by name; their path arguments are checked through the command-spec table.
pub const PATH_CHECKED_COMMANDS: &[&str] =
    &["rm", "rmdir", "cp", "mv", "mkdir", "touch", "ln", "chmod"];

// `safesh` itself is always allowed: `safesh retry` inside a larger script runs under the gate.

/// Options that come from the caller, not the config.
#[derive(Debug, Clone, Default)]
pub struct PolicyOptions {
    /// Agent session id from hook stdin. When present, `allowProjectCommands` defaults to true (D3).
    pub session_id: Option<String>,
}

/// Decides [`Request`]s for one invocation. Built once from the merged config.
pub struct Policy {
    deno: Deno,
    host: HostEnv,
    workspace: Option<String>,
    roots: Vec<String>,
    block_project_dir_write: bool,
    /// User `permissions.write`, absolute in lexical and canonical form.
    explicit_write: Vec<String>,
    /// `permissions.run`, including session grants, for exact-string matches.
    run_set: BTreeSet<String>,
    allow_project_commands: bool,
    rules: Vec<crate::rules::Rule>,
    session_id: Option<String>,
}

impl Policy {
    /// `cwd` is the invocation's base cwd. It fills `${CWD}` and anchors relative list entries.
    pub fn new(
        config: &Config,
        cwd: &Path,
        host: &HostEnv,
        opts: &PolicyOptions,
    ) -> Result<Self, PolicyError> {
        let workspace = workspace_variable(config, cwd, host);
        let perms = config.permissions.clone().unwrap_or_default();
        let block = config.block_project_dir_write == Some(true);
        let roots = workspace_roots(config, host);
        let abs = |list: &[String]| -> Vec<PathBuf> {
            list.iter()
                .map(|p| {
                    let e = expand_path(p, cwd, workspace.as_deref(), host);
                    PathBuf::from(resolve_lexical(&cwd.to_string_lossy(), &e))
                })
                .collect()
        };

        let eff = effective_permissions(config, cwd, host);
        // Roots are checked by `blockProjectDirWrite` logic, not by the injected deny.
        let mut write = eff.write.clone();
        // TS DEFAULT_CONFIG adds existing `~/.dir` dirs to write at startup.
        write.extend(compute_home_dot_dir_defaults(Some(&host.home)));
        let lists = Lists {
            read: with_canonical(&abs(&eff.read)),
            deny_read: with_canonical(&abs(&eff.deny_read)),
            write: with_canonical(&abs(&write)),
            deny_write: {
                let mut deny = abs(perms.deny_write.as_deref().unwrap_or_default());
                deny.extend(protected_dirs(&roots, cwd, host));
                with_canonical(&deny)
            },
            // Only explicit paths go to Deno. Names are resolved lazily per check (D2), since
            // resolving the whole allowlist through PATH up front costs tens of ms.
            run: eff
                .run
                .iter()
                .filter(|r| r.contains('/'))
                .cloned()
                .collect(),
        };
        let deno = Deno::new(&lists, host, cwd)?;
        let explicit_write = with_canonical(&abs(perms.write.as_deref().unwrap_or_default()));

        Ok(Self {
            deno,
            host: host.clone(),
            workspace,
            roots,
            block_project_dir_write: block,
            explicit_write,
            run_set: eff
                .run
                .into_iter()
                .chain(PATH_CHECKED_COMMANDS.iter().map(|c| (*c).to_string()))
                .chain(["safesh".to_string()])
                .collect(),
            rules: config.rules.clone().unwrap_or_default(),
            session_id: opts.session_id.clone(),
            allow_project_commands: config
                .allow_project_commands
                .unwrap_or(opts.session_id.is_some()),
        })
    }

    #[must_use]
    pub fn check(&self, request: &Request) -> Decision {
        match request {
            Request::Run { program, cwd } => self.check_run(&program.to_string_lossy(), cwd),
            Request::Path { path, cwd, access } => self.check_path(path, cwd, *access),
        }
    }

    /// Port of TS `validatePath` returning a decision instead of throwing.
    #[must_use]
    pub fn check_path(&self, path: &Path, cwd: &Path, access: Access) -> Decision {
        let expanded = expand_path(
            &path.to_string_lossy(),
            cwd,
            self.workspace.as_deref(),
            &self.host,
        );
        let real = resolve_path(Path::new(&expanded), cwd);
        if is_device(Path::new(&expanded)) || is_device(&real) {
            return Decision::Allow { resolved: real };
        }
        let state = match access {
            Access::Read => self.deno.query_read(&real),
            Access::Write => self.deno.query_write(&real),
        };
        if state == State::Denied {
            return Decision::Deny {
                resolved: real,
                reason: Reason::Denied,
            };
        }
        if access == Access::Write && self.block_project_dir_write {
            let real_s = real.to_string_lossy();
            if is_path_within_any(&real_s, &self.explicit_write) {
                return Decision::Allow { resolved: real };
            }
            if self.within_roots(&real_s, cwd) {
                return Decision::Deny {
                    resolved: real,
                    reason: Reason::ProjectDirWriteBlocked,
                };
            }
        }
        if state == State::Granted {
            Decision::Allow { resolved: real }
        } else {
            Decision::Ask {
                resolved: real,
                reason: Reason::NotAllowed,
            }
        }
    }

    /// Port of TS `checkCommandPermission`. The basename rule is replaced by D2: a path command
    /// is allowed when it is the program an allowed name resolves to through PATH.
    #[must_use]
    pub fn check_run(&self, command: &str, cwd: &Path) -> Decision {
        let allow = |p: &str| Decision::Allow { resolved: p.into() };
        let ask = |p: &str| Decision::Ask {
            resolved: p.into(),
            reason: Reason::NotAllowed,
        };
        if self.run_set.contains(command) {
            return allow(command);
        }
        if !command.contains('/') {
            return ask(command);
        }

        let cwd_s = cwd.to_string_lossy();
        let abs = resolve_lexical(&cwd_s, command);
        if self.deno.query_run(&abs) == State::Granted
            || self.is_resolved_allowed_name(&abs, cwd)
            || self.is_scratchpad_script(&abs)
        {
            return allow(&abs);
        }

        if command.starts_with('/') {
            if self.allow_project_commands && self.within_roots(command, cwd) {
                return if is_executable_file(Path::new(command)) {
                    allow(command)
                } else {
                    not_found(command)
                };
            }
            return ask(command);
        }

        if is_executable_file(Path::new(&abs)) {
            if (self.allow_project_commands && self.within_roots(&abs, cwd))
                || self.run_set.contains(&abs)
            {
                return allow(&abs);
            }
            return ask(&abs);
        }

        for root in &self.roots {
            let project_path = resolve_lexical(&resolve_lexical(&cwd_s, root), command);
            if is_executable_file(Path::new(&project_path)) {
                if self.allow_project_commands || self.run_set.contains(&project_path) {
                    return allow(&project_path);
                }
                return ask(&project_path);
            }
        }
        not_found(command)
    }

    /// D2: `abs` is allowed when its basename is an allowed name and PATH resolves that name to `abs`.
    fn is_resolved_allowed_name(&self, abs: &str, cwd: &Path) -> bool {
        Path::new(abs)
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|name| self.run_set.contains(name))
            && self.resolves_by_name(abs, cwd)
    }

    /// Whether the session's PATH resolves `abs`'s basename to `abs` itself. The script can't
    /// change this PATH: it is the one the policy was built with.
    pub(crate) fn resolves_by_name(&self, abs: &str, cwd: &Path) -> bool {
        let Some(name) = Path::new(abs).file_name().and_then(|n| n.to_str()) else {
            return false;
        };
        deno_permissions::which::which_in(
            HostSys::new(&self.host, cwd),
            name,
            self.host.path.clone(),
            cwd.to_path_buf(),
        )
        .is_ok_and(|found| found == Path::new(abs))
    }

    /// Scripts in this agent session's own scratchpad are trusted, as in TS. A script the same
    /// command is about to write counts too.
    fn is_scratchpad_script(&self, abs: &str) -> bool {
        let sid = self.session_id.as_deref();
        let uid = Some(self.host.uid);
        crate::scratchpad::is_session_scratchpad_script(abs, sid, uid)
            || crate::scratchpad::is_new_session_scratchpad_script(abs, sid, uid)
    }

    fn within_roots(&self, path: &str, cwd: &Path) -> bool {
        let cwd_s = cwd.to_string_lossy();
        let path = resolve_lexical(&cwd_s, path);
        self.roots
            .iter()
            .any(|root| is_path_within(&path, &resolve_lexical(&cwd_s, root)))
    }
}

/// Dirs holding grants and approvals. The shell may not write them, or an agent could grant
/// itself access. `safesh retry` and `install` write them from outside the gate. Other programs
/// (a Python script, `git checkout`) are outside what safesh sees, as everywhere else.
fn protected_dirs(roots: &[String], cwd: &Path, host: &HostEnv) -> Vec<PathBuf> {
    let mut dirs = vec![
        host.state_dir.clone(),
        crate::config::get_global_config_dir(host),
    ];
    let cwd_s = cwd.to_string_lossy();
    let bases = roots
        .iter()
        .map(|r| PathBuf::from(resolve_lexical(&cwd_s, r)))
        .chain(std::iter::once(cwd.to_path_buf()));
    for base in bases {
        dirs.push(base.join(".temp").join("safesh"));
        dirs.push(base.join(".config").join("safesh"));
    }
    dirs
}

/// Streams and terminal devices that hold no data, so bash redirects to them need no grant.
fn is_device(path: &Path) -> bool {
    const DEVICES: &[&str] = &[
        "/dev/null",
        "/dev/stdin",
        "/dev/stdout",
        "/dev/stderr",
        "/dev/tty",
        "/dev/zero",
        "/dev/random",
        "/dev/urandom",
    ];
    let Some(p) = path.to_str() else {
        return false;
    };
    DEVICES.contains(&p)
        || p.strip_prefix("/dev/fd/")
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

fn not_found(command: &str) -> Decision {
    Decision::Deny {
        resolved: command.into(),
        reason: Reason::NotFound,
    }
}

/// TS `commandExists`: the path exists and is a file.
fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

/// Expanded, absolute permission lists handed to `deno_permissions`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Lists {
    read: Vec<String>,
    deny_read: Vec<String>,
    write: Vec<String>,
    deny_write: Vec<String>,
    run: Vec<String>,
}

/// Thin wrapper over a `PermissionsContainer` that never prompts.
struct Deno {
    container: PermissionsContainer,
}

/// Collapses Deno's states into the three that matter here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Granted,
    Prompt,
    Denied,
}

impl From<PermissionState> for State {
    fn from(s: PermissionState) -> Self {
        match s {
            // GrantedPartial: this path is granted, but some descendants are denied (for example a
            // project root holding `.config/safesh`). Writes that name a denied descendant are
            // still denied on their own.
            PermissionState::Granted | PermissionState::GrantedPartial => Self::Granted,
            PermissionState::Denied | PermissionState::DeniedPartial => Self::Denied,
            PermissionState::Prompt | PermissionState::Ignored => Self::Prompt,
        }
    }
}

impl Deno {
    /// `cwd` anchors relative list entries and PATH lookups for bare run names.
    fn new(lists: &Lists, host: &HostEnv, cwd: &Path) -> Result<Self, PolicyError> {
        let parser = Arc::new(RuntimePermissionDescriptorParser::new(HostSys::new(
            host, cwd,
        )));
        // `Some(vec![])` means "allow all" in Deno, so an empty list must be `None`.
        let some = |v: &Vec<String>| (!v.is_empty()).then(|| v.clone());
        let opts = PermissionsOptions {
            allow_read: some(&lists.read),
            deny_read: some(&lists.deny_read),
            allow_write: some(&lists.write),
            deny_write: some(&lists.deny_write),
            allow_run: some(&lists.run),
            prompt: false,
            ..Default::default()
        };
        let perms = Permissions::from_options(parser.as_ref(), &opts)?;
        Ok(Self {
            container: PermissionsContainer::new(parser, perms),
        })
    }

    /// `path` must be absolute.
    fn query_read(&self, path: &Path) -> State {
        // A non-UTF-8 path can't be queried; `None` would mean "any path".
        let Some(p) = path.to_str() else {
            return State::Prompt;
        };
        self.container
            .query_read(Some(p))
            .map_or(State::Prompt, State::from)
    }

    /// `path` must be absolute.
    fn query_write(&self, path: &Path) -> State {
        let Some(p) = path.to_str() else {
            return State::Prompt;
        };
        self.container
            .query_write(Some(p))
            .map_or(State::Prompt, State::from)
    }

    /// `program` must be an absolute path.
    fn query_run(&self, program: &str) -> State {
        self.container
            .query_run(Some(program))
            .map_or(State::Prompt, State::from)
    }
}

/// Paths in both lexical and canonical form, so entries under a symlinked dir (macOS `/tmp`,
/// `/var`) match canonical query paths.
fn with_canonical(paths: &[PathBuf]) -> Vec<String> {
    let mut out = Vec::with_capacity(paths.len() * 2);
    for p in paths {
        out.push(p.to_string_lossy().into_owned());
        if let Ok(c) = soft_canonicalize::soft_canonicalize(p)
            && c != *p
        {
            out.push(c.to_string_lossy().into_owned());
        }
    }
    dedupe(out)
}
