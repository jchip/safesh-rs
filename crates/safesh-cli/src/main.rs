//! safesh command line entry point.

mod approval;
mod cache;
mod context;
mod evaluate;
mod gate;
mod hook;
mod install;
mod messages;
mod retry;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::{Parser, Subcommand};

use context::Context;
use gate::PolicyGate;

#[derive(Parser)]
#[command(
    name = "safesh",
    version,
    about = "A bash runner for AI agents with a guard at every exec"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Agent `PreToolUse` hook. Reads hook JSON on stdin.
    Hook {
        /// claude, gemini, codex or agy.
        agent: safesh_hooks::Agent,
        /// Log the decision without acting on it.
        #[arg(long)]
        shadow: bool,
        /// Most seconds to wait for the user. `safesh install` sets it below the hook timeout.
        #[arg(long)]
        budget_sec: Option<u64>,
    },
    /// Run a cached, approved script.
    Run {
        #[arg(long)]
        id: String,
        /// Agent session id, for session grants.
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        background: bool,
        /// Pending id whose once-grant applies to this run.
        #[arg(long)]
        pending: Option<String>,
    },
    /// Nested shell entry: `bash`/`sh` re-routed by the runtime gate.
    #[command(hide = true)]
    Shell {
        #[arg(long, default_value = "bash")]
        shell_name: String,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Approve a pending command, then run its script.
    Retry {
        #[arg(long)]
        id: String,
        /// 1 once, 2 always (this project), 3 this session, 4 deny, 5 always (all projects).
        #[arg(long)]
        choice: String,
    },
    /// Approve a pending path, then run its script.
    RetryPath {
        #[arg(long)]
        id: String,
        /// `r`, `w` or `rw`, then 1 once, 2 session, 3 always, optional `d` for the directory; or `4`/`deny`.
        #[arg(long)]
        choice: String,
    },
    /// Approval picker shown inside a tmux popup.
    #[command(hide = true)]
    ApproveUi {
        #[arg(long)]
        id: String,
        #[arg(long)]
        state_dir: PathBuf,
        #[arg(long)]
        deadline_ms: u64,
    },
    /// Show one approval prompt and print the choice. For tests.
    #[command(hide = true)]
    ApproveAsk {
        #[arg(long)]
        id: String,
        /// Commands to approve. With `--path`, the first one is shown as the path's command.
        #[arg(long = "command")]
        commands: Vec<String>,
        #[arg(long)]
        path: Option<String>,
        #[arg(long)]
        write: bool,
        #[arg(long, value_delimiter = ',', default_value = "tmux,gui")]
        channels: Vec<String>,
        #[arg(long, default_value_t = 60)]
        show_sec: u64,
        #[arg(long, default_value_t = 120)]
        give_up_sec: u64,
        #[arg(long)]
        bin_dir: Vec<PathBuf>,
    },
    /// Show what the pre-check decides for a script.
    Check {
        script: String,
        #[arg(long)]
        session: Option<String>,
    },
    /// Install the hook for an agent.
    Install {
        /// claude, codex or agy.
        agent: String,
        /// Print the settings change without writing it.
        #[arg(long)]
        dry_run: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let code = match cli.cmd {
        Cmd::Hook {
            agent,
            shadow,
            budget_sec,
        } => hook::run(agent, shadow, budget_sec),
        Cmd::Run {
            id,
            session,
            background,
            pending,
        } => run_cached(&id, session.as_deref(), background, pending.as_deref()),
        Cmd::Shell { shell_name, args } => shell(&shell_name, &args),
        Cmd::Retry { id, choice } => retry::retry(&id, &choice),
        Cmd::RetryPath { id, choice } => retry::retry_path(&id, &choice),
        Cmd::ApproveUi {
            id,
            state_dir,
            deadline_ms,
        } => approval::ui::run(&id, &state_dir, deadline_ms),
        Cmd::ApproveAsk {
            id,
            commands,
            path,
            write,
            channels,
            show_sec,
            give_up_sec,
            bin_dir,
        } => {
            use safesh_policy::state::pending::PathOperation;
            let request = match path {
                Some(path) => approval::Request::Path {
                    path,
                    op: if write {
                        PathOperation::Write
                    } else {
                        PathOperation::Read
                    },
                    command: commands.into_iter().next(),
                },
                None => approval::Request::Command {
                    commands,
                    command: None,
                },
            };
            let timing = approval::Timing {
                show: std::time::Duration::from_secs(show_sec),
                give_up_at: std::time::Instant::now() + std::time::Duration::from_secs(give_up_sec),
            };
            approval::cli_ask(&id, request, &channels, timing, bin_dir)
        }
        Cmd::Check { script, session } => check(&script, session.as_deref()),
        Cmd::Install { agent, dry_run } => install::install(&agent, dry_run),
    };
    ExitCode::from(code)
}

fn fail(msg: &str) -> u8 {
    eprintln!("safesh: {msg}");
    1
}

fn self_exe() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("safesh"))
}

/// Runs `args` under a gate built from `ctx`.
pub(crate) fn execute(
    ctx: Context,
    args: safesh_exec::ShellArgs,
    script_id: Option<String>,
    background: bool,
) -> u8 {
    let nested_env = ctx.nested_env(script_id.as_deref());
    let gate = Arc::new(PolicyGate {
        ctx: Arc::new(ctx),
        script_id,
        run_in_background: background,
    });
    let config = safesh_exec::RunConfig {
        args,
        self_exe: self_exe(),
        nested_env,
    };
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => return fail(&e.to_string()),
    };
    match rt.block_on(safesh_exec::run(config, gate)) {
        Ok(code) => code,
        Err(e) => fail(&e.to_string()),
    }
}

fn command_args(script: String) -> safesh_exec::ShellArgs {
    safesh_exec::ShellArgs {
        shell_name: "safesh".into(),
        command: Some(script),
        script: None,
        positional: Vec::new(),
        set_options: Vec::new(),
    }
}

fn run_cached(id: &str, session: Option<&str>, background: bool, pending: Option<&str>) -> u8 {
    let cwd = match std::env::current_dir() {
        Ok(c) => c,
        Err(e) => return fail(&e.to_string()),
    };
    let session = session
        .map(str::to_string)
        .or_else(|| std::env::var(context::ENV_SESSION).ok());
    let host = safesh_policy::host::HostEnv::from_process();
    let grant = match pending {
        Some(p) => match approval::once::claim(&host.state_dir, p, id) {
            Some(g) => Some(g),
            None => {
                return fail(&format!(
                    "no once-grant {p} for this script; it may have been used or expired"
                ));
            }
        },
        None => None,
    };
    let ctx = match Context::load_granted(host, &cwd, session.as_deref(), |c| {
        if let Some(g) = grant {
            g.apply(c);
        }
    }) {
        Ok(c) => c,
        Err(e) => return fail(&e),
    };
    let script = match cache::load(&ctx.host, id) {
        Ok(s) => s,
        Err(e) => return fail(&e),
    };
    execute(ctx, command_args(script), Some(id.to_string()), background)
}

fn shell(name: &str, args: &[String]) -> u8 {
    let args = match safesh_exec::parse_bash_args(name, args) {
        Ok(a) => a,
        Err(e) => return fail(&e),
    };
    let ctx = match Context::from_env() {
        Ok(c) => c,
        Err(e) => return fail(&e),
    };
    let script_id = std::env::var(context::ENV_SCRIPT).ok();
    execute(ctx, args, script_id, false)
}

fn check(script: &str, session: Option<&str>) -> u8 {
    let cwd = match std::env::current_dir() {
        Ok(c) => c,
        Err(e) => return fail(&e.to_string()),
    };
    let ctx = match Context::load(&cwd, session) {
        Ok(c) => c,
        Err(e) => return fail(&e),
    };
    for w in &ctx.warnings {
        println!("warning: {w}");
    }
    if safesh_analyze::is_control_plane(script) {
        println!("control plane: passes through");
        return 0;
    }
    match evaluate::evaluate(&ctx, script) {
        Ok(ev) => {
            for e in &ev.analysis.execs {
                println!(
                    "exec {:?} cwd={:?} builtin={} function={}",
                    e.argv, e.cwd, e.builtin, e.function
                );
            }
            for o in &ev.analysis.opens {
                println!(
                    "open {:?} {:?} {:?} cwd={:?}",
                    o.path, o.access, o.kind, o.cwd
                );
            }
            println!("final cwd: {:?}", ev.final_cwd(&ctx.base_cwd));
            if ev.is_allowed() {
                println!("decision: allow");
            } else {
                for (what, p) in &ev.problems {
                    println!("problem at {what}: {p:?}");
                }
                println!("decision: ask");
            }
            0
        }
        Err(e) => fail(&e),
    }
}
