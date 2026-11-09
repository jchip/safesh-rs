//! brush-core embedding: runs a script with a policy gate on every exec and file open.
//!
//! Nested `bash`, `sh` and `dash` are re-routed to `safesh shell`, so their commands hit the gate too.
//! A denial prints the gate's message and stops the whole run with status 126.

mod args;

use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use brush_builtins::{BuiltinSet, ShellBuilderExt as _};
use brush_core::extensions::{DefaultProcessLauncher, ProcessLauncher, ShellExtensionsImpl};
use brush_core::processes::{ChildProcess, LaunchRequest};

pub use args::{ShellArgs, parse_bash_args};
pub use safesh_policy::effect::Access;

/// Exit status for a run stopped by policy.
pub const DENIED_STATUS: u8 = 126;

/// One exec the shell is about to start.
#[derive(Debug, Clone)]
pub struct ExecRequest<'a> {
    /// Program as resolved by the shell; may be relative to `cwd`.
    pub program: &'a Path,
    /// `argv[0]` as the process will see it, then the arguments.
    pub argv: Vec<String>,
    pub cwd: &'a Path,
}

/// Decides execs and opens. On a denial it returns the text to show the user; it is responsible
/// for any side effects such as writing a pending file.
pub trait Gate: Send + Sync {
    fn check_exec(&self, req: &ExecRequest<'_>) -> Result<(), String>;
    /// `path` is absolute.
    fn check_open(&self, path: &Path, access: Access) -> Result<(), String>;
}

/// Settings shared by every launch in one run.
#[derive(Clone)]
struct Ctx {
    gate: Arc<dyn Gate>,
    self_exe: PathBuf,
    /// Env vars passed to nested shells so they rebuild the same policy.
    nested_env: Vec<(String, String)>,
}

/// `Default` is required by brush. A launcher without a context denies everything.
#[derive(Clone, Default)]
struct PolicyLauncher {
    ctx: Option<Arc<Ctx>>,
}

type Extensions = ShellExtensionsImpl<
    brush_core::extensions::DefaultErrorFormatter,
    PolicyLauncher,
    brush_core::extensions::DefaultTaskSpawner,
>;

impl PolicyLauncher {
    fn ctx(&self) -> Result<&Ctx, io::Error> {
        self.ctx
            .as_deref()
            .ok_or_else(|| abort("[SAFESH] BLOCKED: no policy loaded"))
    }

    fn check_exec(&self, req: &LaunchRequest) -> Result<(), io::Error> {
        let ctx = self.ctx()?;
        let mut argv = Vec::with_capacity(req.args.len() + 1);
        argv.push(req.argv0.clone());
        argv.extend(req.args.iter().cloned());
        let exec = ExecRequest {
            program: &req.program,
            argv,
            cwd: &req.working_dir,
        };
        ctx.gate.check_exec(&exec).map_err(|m| abort(&m))
    }

    /// Re-route nested shells to `safesh shell`, keeping their arguments.
    fn reroute_shell(&self, req: &mut LaunchRequest) {
        let Some(ctx) = self.ctx.as_deref() else {
            return;
        };
        // Windows: `C:\Git\usr\bin\bash.exe` is `bash`, so it is rerouted too.
        let is_shell = matches!(
            safesh_policy::paths::command_name(&req.program.to_string_lossy()),
            "bash" | "sh" | "dash"
        );
        if !is_shell {
            return;
        }
        req.program.clone_from(&ctx.self_exe);
        let mut args = vec![
            "shell".to_string(),
            "--shell-name".into(),
            req.argv0.clone(),
            "--".into(),
        ];
        args.append(&mut req.args);
        req.args = args;
        req.argv0 = "safesh".into();
        let chain = match std::env::var(ENV_PARENT_PIDS) {
            Ok(c) if !c.is_empty() => format!("{c}:{}", std::process::id()),
            _ => std::process::id().to_string(),
        };
        let parent = (ENV_PARENT_PIDS.to_string(), chain);
        for (k, v) in ctx.nested_env.iter().chain(std::iter::once(&parent)) {
            req.env.retain(|(ek, _)| ek != k);
            req.env.push((k.clone(), v.clone()));
        }
    }
}

/// Set on nested shells: pids of every safesh above them, outermost first, `:`-separated.
const ENV_PARENT_PIDS: &str = "SAFESH_PARENT_PIDS";

/// Prints the denial and stops the run. Children in a pipeline see a closed pipe and exit.
fn abort(message: &str) -> io::Error {
    let _ = io::stdout().flush();
    let mut err = io::stderr().lock();
    let _ = writeln!(err, "{message}");
    let _ = err.flush();
    // A nested shell's denial must stop every safesh above it, not just this child.
    stop_parents();
    std::process::exit(i32::from(DENIED_STATUS));
}

/// Pids of the safesh processes above this one, outermost first.
#[cfg(any(unix, windows))]
fn parent_pids() -> Vec<u32> {
    std::env::var(ENV_PARENT_PIDS)
        .unwrap_or_default()
        .split(':')
        .filter_map(|p| p.parse::<u32>().ok())
        .filter(|&p| p > 0)
        .collect()
}

/// Unix: SIGUSR1 to each parent, whose handler exits it with status 126 (see `exit_on_child_denial`).
#[cfg(unix)]
fn stop_parents() {
    for pid in parent_pids() {
        let Some(pid) = i32::try_from(pid)
            .ok()
            .and_then(rustix::process::Pid::from_raw)
        else {
            continue;
        };
        let _ = rustix::process::kill_process(pid, rustix::process::Signal::USR1);
    }
}

/// Windows has no signal a process can catch, so each parent is terminated with status 126.
/// Outermost first, and each kill waits until that process is gone: a parent never sees its
/// child exit while a safesh above it still runs, so no later command of any script can start.
#[cfg(windows)]
fn stop_parents() {
    /// Bound on each wait, so a parent stuck in the kernel can't hang the denial.
    const WAIT_MS: u32 = 2000;
    for pid in parent_pids() {
        safesh_policy::compat::terminate_and_wait(pid, u32::from(DENIED_STATUS), WAIT_MS);
    }
}

#[cfg(not(any(unix, windows)))]
fn stop_parents() {}

/// SIGUSR1 from a denied nested shell exits this process with status 126 right in the signal
/// handler, so no later command of this script can run first. Windows needs no setup: the child
/// terminates this process directly.
#[cfg(not(unix))]
fn exit_on_child_denial() {}

#[cfg(unix)]
fn exit_on_child_denial() {
    let always = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let _ = signal_hook::flag::register_conditional_shutdown(
        signal_hook::consts::SIGUSR1,
        i32::from(DENIED_STATUS),
        always,
    );
}

impl ProcessLauncher for PolicyLauncher {
    fn spawn(&self, mut request: LaunchRequest) -> Result<ChildProcess, io::Error> {
        self.check_exec(&request)?;
        self.reroute_shell(&mut request);
        DefaultProcessLauncher.spawn(request)
    }

    fn exec(&self, mut request: LaunchRequest) -> io::Error {
        if let Err(e) = self.check_exec(&request) {
            return e;
        }
        self.reroute_shell(&mut request);
        DefaultProcessLauncher.exec(request)
    }

    fn host_path(&self, path: &Path) -> Option<std::path::PathBuf> {
        safesh_policy::paths::host_path(path)
    }

    fn check_open(&self, path: &Path, write: bool) -> Result<(), io::Error> {
        let ctx = self.ctx()?;
        let access = if write { Access::Write } else { Access::Read };
        ctx.gate.check_open(path, access).map_err(|m| abort(&m))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ExecError {
    #[error("shell setup failed: {0}")]
    Setup(String),
}

/// What to run and how.
pub struct RunConfig {
    pub args: ShellArgs,
    /// Path to the running safesh binary, for nested shells.
    pub self_exe: PathBuf,
    /// Env vars nested shells need to rebuild the same policy.
    pub nested_env: Vec<(String, String)>,
}

/// Runs a script under the gate and returns its exit status.
pub async fn run(config: RunConfig, gate: Arc<dyn Gate>) -> Result<u8, ExecError> {
    let RunConfig {
        args,
        self_exe,
        nested_env,
    } = config;
    exit_on_child_denial();
    let launcher = PolicyLauncher {
        ctx: Some(Arc::new(Ctx {
            gate,
            self_exe,
            nested_env,
        })),
    };
    let positional = args.positional.clone();
    let mut shell = brush_core::Shell::builder_with_extensions::<Extensions>()
        .process_launcher(launcher)
        .profile(brush_core::ProfileLoadBehavior::Skip)
        .rc(brush_core::RcLoadBehavior::Skip)
        .command_string_mode(args.command.is_some())
        .shell_name(args.shell_name.clone())
        .shell_args(positional.clone())
        .default_builtins(BuiltinSet::BashMode)
        .build()
        .await
        .map_err(|e| ExecError::Setup(e.to_string()))?;

    if !args.set_options.is_empty() {
        let set_cmd = format!("set {}", args.set_options.join(" "));
        let params = shell.default_exec_params();
        let _ = shell
            .run_string(set_cmd, &brush_core::SourceInfo::default(), &params)
            .await
            .map_err(|e| ExecError::Setup(e.to_string()))?;
    }

    let result = match (&args.command, &args.script) {
        (Some(cmd), _) => shell.run_dash_c_command(cmd.clone()).await,
        (None, Some(script)) => shell.run_script(script, positional.iter().cloned()).await,
        (None, None) => {
            let mut text = String::new();
            io::Read::read_to_string(&mut io::stdin(), &mut text)
                .map_err(|e| ExecError::Setup(e.to_string()))?;
            shell.run_dash_c_command(text).await
        }
    };
    match result {
        Ok(r) => Ok(u8::from(r.exit_code)),
        Err(e) => {
            eprintln!("safesh: {e}");
            Ok(1)
        }
    }
}
