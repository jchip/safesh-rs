//! `safesh hook <agent>`: the `PreToolUse` entry point.

use std::fmt::Write as _;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use safesh_hooks::{
    Agent, ApprovalRequest, HookInput, Outcome, consume_approval, format_output,
    format_permission_request_allow, parse_input, write_approval,
};
use safesh_policy::effect::Access;
use safesh_policy::policy::Problem;
use safesh_policy::state::pending::{
    PathOperation, PendingCommand, PendingPathRequest, generate_pending_id, write_pending_command,
    write_pending_path,
};

use crate::context::Context;
use crate::evaluate::{Evaluation, evaluate};
use crate::gate::now_iso;
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

pub fn run(agent: Agent, shadow: bool) -> u8 {
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
    let outcome = match decide(agent, &input) {
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

/// Decides one hook call. Public for `safesh check`-style tests.
pub fn decide(agent: Agent, input: &HookInput) -> Result<Outcome, String> {
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
        let command = rewrite(&ctx, &ev, &id, input);
        if agent == Agent::Codex {
            let req = approval_request(input, &command);
            write_approval(&ctx.host, &req, now_ms()).map_err(|e| e.to_string())?;
        }
        return Ok(Outcome::Rewrite { command });
    }
    Ok(Outcome::Deny {
        message: block(&ctx, &ev, &id, input),
    })
}

fn rewrite(ctx: &Context, ev: &Evaluation, id: &str, input: &HookInput) -> String {
    let mut cmd = format!("{} run --id {id}", sh_quote(&self_exe().to_string_lossy()));
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

/// Writes a pending approval and returns the deny text.
fn block(ctx: &Context, ev: &Evaluation, id: &str, input: &HookInput) -> String {
    if let Some((what, p)) = ev.problems.iter().find(|(_, p)| p.is_hard()) {
        let why = match p {
            Problem::Run { .. } => "not found or on a deny list",
            Problem::Path { .. } => "on a deny list",
            Problem::Rule { .. } => "denied by a rule",
            Problem::Ask(r) => r.as_str(),
        };
        return messages::denied(what, &[why.to_string()]);
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
            Ok(()) => messages::path_blocked(
                &pending.path,
                *access == Access::Write,
                Path::new(&pending.path).is_dir(),
                &pending.id,
            ),
            Err(e) => messages::denied(&pending.path, &[e.to_string()]),
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
        Ok(()) => messages::blocked(&commands, &pending.id, &reasons),
        Err(e) => messages::denied(&commands.join(", "), &[e.to_string()]),
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
}
