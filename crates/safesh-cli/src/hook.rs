//! `safesh hook <agent>`: the `PreToolUse` entry point.

use std::fmt::Write as _;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use safesh_hooks::{
    Agent, ApprovalRequest, HookInput, Outcome, consume_approval, format_output,
    format_permission_request_allow, parse_input, write_approval,
};
use safesh_policy::effect::Access;
use safesh_policy::policy::Problem;
use safesh_policy::state::pending::{
    PathOperation, PendingCommand, PendingKind, PendingPathRequest, delete_pending,
    generate_pending_id, write_pending_command, write_pending_path,
};

use crate::approval::once::{self, OnceGrant};
use crate::approval::{self, Choice, Request};
use crate::context::Context;
use crate::evaluate::{Evaluation, evaluate};
use crate::gate::now_iso;
use crate::retry::{Scope, save_command_grant, save_path_grant};
use crate::{cache, messages, self_exe};

/// Single-quotes `s` for a POSIX shell.
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// Hook budget when the hook command has no `--budget-sec`. Older installs set no timeout
/// for Claude (default 600 s) and 30 s for the others.
fn default_budget(agent: Agent) -> u64 {
    if agent == Agent::Claude { 590 } else { 25 }
}

pub fn run(agent: Agent, shadow: bool, budget_sec: Option<u64>) -> u8 {
    fail_closed(shadow, || run_inner(agent, shadow, budget_sec))
}

/// Runs the hook body so that a panic cannot let the tool call through. A process that dies
/// from a panic exits 101, which agents treat as "hook errored, carry on": the command would
/// run unchecked. Exit 2 blocks it instead. Shadow mode never blocks, so it passes through.
fn fail_closed(shadow: bool, body: impl FnOnce() -> u8 + std::panic::UnwindSafe) -> u8 {
    std::panic::catch_unwind(body).unwrap_or_else(|_| {
        eprintln!("safesh hook: internal error (panic); blocking the command");
        if shadow { 0 } else { 2 }
    })
}

fn run_inner(agent: Agent, shadow: bool, budget_sec: Option<u64>) -> u8 {
    let mut stdin = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut stdin) {
        eprintln!("safesh hook: cannot read stdin: {e}");
        return 2;
    }
    let input = match parse_input(agent, &stdin) {
        Ok(Some(i)) => i,
        Ok(None) => return emit(agent, None, &Outcome::Passthrough),
        // Fail closed: exit 2 blocks the tool call and shows stderr.
        Err(e) => {
            eprintln!("safesh hook: {e}");
            return 2;
        }
    };
    if input.hook_event_name.as_deref() == Some("PermissionRequest") {
        return permission_request(&input);
    }
    // Shadow mode only logs, so it never shows a prompt.
    let budget =
        (!shadow).then(|| Duration::from_secs(budget_sec.unwrap_or_else(|| default_budget(agent))));
    let outcome = match decide(agent, &input, budget) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("safesh hook: {e}");
            if shadow {
                return emit(agent, Some(&input), &Outcome::Passthrough);
            }
            return 2;
        }
    };
    if shadow {
        log_shadow(agent, &input, &outcome);
        return emit(agent, Some(&input), &Outcome::Passthrough);
    }
    emit(agent, Some(&input), &outcome)
}

fn emit(agent: Agent, input: Option<&HookInput>, outcome: &Outcome) -> u8 {
    let empty = HookInput::default();
    let r = format_output(agent, input.unwrap_or(&empty), outcome);
    if !r.stdout.is_empty() {
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(r.stdout.as_bytes());
        let _ = out.flush();
    }
    u8::try_from(r.exit_code).unwrap_or(1)
}

fn hook_cwd(input: &HookInput) -> Result<PathBuf, String> {
    if let Some(c) = &input.cwd {
        return Ok(c.clone());
    }
    if let Some(c) = std::env::var_os("BASH_PREHOOK_CWD") {
        return Ok(PathBuf::from(c));
    }
    std::env::current_dir().map_err(|e| e.to_string())
}

/// Codex asks again before running a rewritten command; allow it once if the hook rewrote it.
fn permission_request(input: &HookInput) -> u8 {
    let host = safesh_policy::host::HostEnv::from_process();
    let req = approval_request(input, &input.command);
    match consume_approval(&host, &req, now_ms()) {
        Ok(true) => {
            print!("{}", format_permission_request_allow());
            0
        }
        Ok(false) => 0,
        Err(e) => {
            eprintln!("safesh hook: {e}");
            0
        }
    }
}

fn approval_request(input: &HookInput, command: &str) -> ApprovalRequest {
    ApprovalRequest {
        session_id: input.session_id.clone().unwrap_or_default(),
        turn_id: input.turn_id.clone().unwrap_or_default(),
        cwd: input
            .cwd
            .as_ref()
            .map(|c| c.to_string_lossy().into_owned())
            .unwrap_or_default(),
        command: command.to_string(),
    }
}

/// Decides one hook call. With a `budget`, a blocked call that the user can approve is
/// asked through the approval channels first.
pub fn decide(
    agent: Agent,
    input: &HookInput,
    budget: Option<Duration>,
) -> Result<Outcome, String> {
    let script = input.command.as_str();
    if safesh_analyze::is_control_plane(script) {
        return Ok(Outcome::Passthrough);
    }
    let cwd = hook_cwd(input)?;
    let ctx = Context::load(&cwd, input.session_id.as_deref())?;
    let ev = match evaluate(&ctx, script) {
        Ok(ev) => ev,
        Err(e) => {
            return Ok(Outcome::Deny {
                message: format!("[SAFESH] cannot parse this script: {e}"),
            });
        }
    };
    let id = cache::store(&ctx.host, script)?;
    if ev.is_allowed() {
        return rewritten(agent, &ctx, &ev, &id, input, None);
    }
    let blocked = block(&ctx, &ev, &id, input);
    if let (Some(budget), Some(ask)) = (budget, blocked.ask)
        && let Some(choice) = ask_user(&ctx, &ask, budget)
    {
        return Ok(answered(agent, &ctx, &ev, &id, input, &ask, choice));
    }
    Ok(Outcome::Deny {
        message: blocked.message,
    })
}

fn rewritten(
    agent: Agent,
    ctx: &Context,
    ev: &Evaluation,
    id: &str,
    input: &HookInput,
    pending: Option<&str>,
) -> Result<Outcome, String> {
    let command = rewrite(ctx, ev, id, input, pending);
    if agent == Agent::Codex {
        let req = approval_request(input, &command);
        write_approval(&ctx.host, &req, now_ms()).map_err(|e| e.to_string())?;
    }
    Ok(Outcome::Rewrite { command })
}

/// A pending approval the user may answer through a channel.
struct Ask {
    pending_id: String,
    kind: PendingKind,
    request: Request,
}

fn ask_user(ctx: &Context, ask: &Ask, budget: Duration) -> Option<Choice> {
    let env = approval::Env::from_process(&ctx.host.state_dir);
    let prompt = approval::Prompt {
        id: ask.pending_id.clone(),
        cwd: ctx.base_cwd.to_string_lossy().into_owned(),
        request: ask.request.clone(),
    };
    let timing = approval::Timing {
        show: ctx.approval.timeout,
        give_up_at: Instant::now() + budget,
    };
    approval::ask_user(&ctx.approval.channels, &env, &prompt, timing)
}

/// Applies the user's answer. Every allow also writes a once-grant, so this run can't block
/// on the same thing even when a saved grant doesn't take effect (as `retry` does).
fn answered(
    agent: Agent,
    ctx: &Context,
    ev: &Evaluation,
    id: &str,
    input: &HookInput,
    ask: &Ask,
    choice: Choice,
) -> Outcome {
    let host = &ctx.host;
    delete_pending(host, &ask.pending_id, ask.kind);
    let what = match &ask.request {
        Request::Command { commands, .. } => commands.join(", "),
        Request::Path { path, .. } => path.clone(),
    };
    let scope = match choice {
        Choice::Deny => {
            return Outcome::Deny {
                message: format!("[SAFESH] DENIED by the user: {what}"),
            };
        }
        Choice::Once => Scope::Once,
        Choice::Project => Scope::Project,
        Choice::Session => Scope::Session,
        Choice::Global => Scope::Global,
    };
    let session = input.session_id.as_deref();
    let mut grant = OnceGrant {
        script_id: id.to_string(),
        ..OnceGrant::default()
    };
    let saved = match &ask.request {
        Request::Command { commands, .. } => {
            grant.commands.clone_from(commands);
            save_command_grant(
                host,
                commands,
                &ctx.base_cwd,
                &ctx.project_dir,
                session,
                scope,
            )
        }
        Request::Path { path, op, .. } => {
            let paths = vec![path.clone()];
            if *op == PathOperation::Read {
                grant.read = paths;
            } else {
                grant.write = paths;
            }
            save_path_grant(
                host,
                &grant.read,
                &grant.write,
                &ctx.base_cwd,
                session,
                scope,
            )
        }
    };
    let done = saved
        .and_then(|_| once::write(&host.state_dir, &ask.pending_id, &grant))
        .and_then(|()| rewritten(agent, ctx, ev, id, input, Some(&ask.pending_id)));
    done.unwrap_or_else(|e| Outcome::Deny {
        message: messages::denied(&what, &[e]),
    })
}

fn rewrite(
    ctx: &Context,
    ev: &Evaluation,
    id: &str,
    input: &HookInput,
    pending: Option<&str>,
) -> String {
    let mut cmd = format!("{} run --id {id}", sh_quote(&self_exe().to_string_lossy()));
    if let Some(p) = pending {
        let _ = write!(cmd, " --pending {p}");
    }
    if let Some(s) = &input.session_id {
        let _ = write!(cmd, " --session {}", sh_quote(s));
    }
    if input.run_in_background {
        cmd.push_str(" --background");
    } else if let Some(dir) = ev.final_cwd(&ctx.base_cwd) {
        let _ = write!(cmd, " && cd {}", sh_quote(&dir.to_string_lossy()));
    }
    cmd
}

struct Blocked {
    message: String,
    /// Set when a pending file was written, so the user can answer through a channel.
    ask: Option<Ask>,
}

impl Blocked {
    fn text(message: String) -> Self {
        Self { message, ask: None }
    }
}

/// Writes a pending approval and returns the deny text.
fn block(ctx: &Context, ev: &Evaluation, id: &str, input: &HookInput) -> Blocked {
    if let Some((what, p)) = ev.problems.iter().find(|(_, p)| p.is_hard()) {
        let why = match p {
            Problem::Run { .. } => "not found or on a deny list",
            Problem::Path { .. } => "on a deny list",
            Problem::Rule { .. } => "denied by a rule",
            Problem::Ask(r) => r.as_str(),
        };
        return Blocked::text(messages::denied(what, &[why.to_string()]));
    }
    let cwd = ctx.base_cwd.to_string_lossy().into_owned();
    let paths_only = ev
        .problems
        .iter()
        .all(|(_, p)| matches!(p, Problem::Path { .. }));
    if paths_only && let Some((_, Problem::Path { path, access, .. })) = ev.problems.first() {
        let pending = PendingPathRequest {
            id: generate_pending_id(),
            path: path.to_string_lossy().into_owned(),
            operation: match access {
                Access::Read => PathOperation::Read,
                Access::Write => PathOperation::Write,
            },
            cwd,
            script_hash: id.to_string(),
            run_in_background: input.run_in_background.then_some(true),
            session_id: input.session_id.clone(),
            created_at: now_iso(),
        };
        return match write_pending_path(&ctx.host, &pending) {
            Ok(()) => Blocked {
                message: messages::path_blocked(
                    &pending.path,
                    *access == Access::Write,
                    Path::new(&pending.path).is_dir(),
                    &pending.id,
                ),
                ask: Some(Ask {
                    pending_id: pending.id.clone(),
                    kind: PendingKind::Path,
                    request: Request::Path {
                        path: pending.path.clone(),
                        op: pending.operation,
                        command: Some(input.command.clone()),
                    },
                }),
            },
            Err(e) => Blocked::text(messages::denied(&pending.path, &[e.to_string()])),
        };
    }
    let mut commands: Vec<String> = Vec::new();
    let mut reasons = Vec::new();
    for (what, p) in &ev.problems {
        match p {
            Problem::Run { command, .. } => commands.push(command.clone()),
            Problem::Path { path, access, .. } => {
                let verb = if *access == Access::Write {
                    "writes"
                } else {
                    "reads"
                };
                reasons.push(format!("{what} {verb} {}", path.display()));
            }
            Problem::Rule { .. } => {
                commands.push(what.clone());
                reasons.push(format!("{what} matches an ask rule"));
            }
            Problem::Ask(r) => {
                commands.push(what.clone());
                reasons.push(r.clone());
            }
        }
    }
    commands.dedup();
    let pending = PendingCommand {
        id: generate_pending_id(),
        script_hash: id.to_string(),
        commands: commands.clone(),
        cwd,
        project_dir: Some(ctx.project_dir.to_string_lossy().into_owned()),
        timeout: input.timeout,
        run_in_background: input.run_in_background.then_some(true),
        session_id: input.session_id.clone(),
        created_at: now_iso(),
    };
    match write_pending_command(&ctx.host, &pending) {
        Ok(()) => Blocked {
            message: messages::blocked(&commands, &pending.id, &reasons),
            ask: Some(Ask {
                pending_id: pending.id,
                kind: PendingKind::Command,
                request: Request::Command {
                    commands,
                    command: Some(input.command.clone()),
                },
            }),
        },
        Err(e) => Blocked::text(messages::denied(&commands.join(", "), &[e.to_string()])),
    }
}

fn log_shadow(agent: Agent, input: &HookInput, outcome: &Outcome) {
    let host = safesh_policy::host::HostEnv::from_process();
    let (kind, detail) = match outcome {
        Outcome::Passthrough => ("passthrough", String::new()),
        Outcome::Rewrite { command } => ("rewrite", command.clone()),
        Outcome::Deny { message } => ("deny", message.clone()),
    };
    let line = serde_json::json!({
        "ts": now_iso(),
        "agent": format!("{agent:?}"),
        "cwd": input.cwd,
        "command": input.command,
        "decision": kind,
        "detail": detail,
    });
    let path = host.state_dir.join("shadow.jsonl");
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(f, "{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::sh_quote;

    #[test]
    fn quoting() {
        assert_eq!(sh_quote("a b"), "'a b'");
        assert_eq!(sh_quote("it's"), "'it'\\''s'");
    }

    #[test]
    fn a_panic_in_the_hook_blocks_the_command() {
        // Keep the expected panic out of the test output.
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let blocked = super::fail_closed(false, || panic!("boom"));
        let shadow = super::fail_closed(true, || panic!("boom"));
        std::panic::set_hook(prev);
        assert_eq!(blocked, 2);
        assert_eq!(shadow, 0);
        assert_eq!(super::fail_closed(false, || 0), 0);
    }
}
