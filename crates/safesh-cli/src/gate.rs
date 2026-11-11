//! Runtime gate: turns policy problems into pending files and user-facing messages.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use safesh_exec::{Access, ExecRequest, Gate};
use safesh_policy::effect::{Decision, Reason};
use safesh_policy::policy::Problem;
use safesh_policy::state::pending::{
    PathOperation, PendingCommand, PendingPathRequest, generate_pending_id, write_pending_command,
    write_pending_path,
};

use crate::context::Context;
use crate::messages;

pub struct PolicyGate {
    pub ctx: Arc<Context>,
    /// Cached script this run executes. Without it there is nothing to retry.
    pub script_id: Option<String>,
    pub run_in_background: bool,
}

pub fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

impl PolicyGate {
    fn block_command(&self, commands: &[String], cwd: &Path, reasons: &[String]) -> String {
        let Some(script) = &self.script_id else {
            return messages::denied(&commands.join(", "), reasons);
        };
        let pending = PendingCommand {
            id: generate_pending_id(),
            script_hash: script.clone(),
            commands: commands.to_vec(),
            cwd: cwd.to_string_lossy().into_owned(),
            project_dir: Some(self.ctx.project_dir.to_string_lossy().into_owned()),
            timeout: None,
            run_in_background: self.run_in_background.then_some(true),
            session_id: self.ctx.session_id.clone(),
            created_at: now_iso(),
        };
        match write_pending_command(&self.ctx.host, &pending) {
            Ok(()) => messages::blocked(commands, &pending.id, reasons),
            Err(e) => messages::denied(
                &commands.join(", "),
                &[format!("cannot save pending approval: {e}")],
            ),
        }
    }

    fn block_path(&self, path: &Path, access: Access, cwd: &Path) -> String {
        let Some(script) = &self.script_id else {
            return messages::denied(
                &path.to_string_lossy(),
                &["path is outside the allowed lists".into()],
            );
        };
        let pending = PendingPathRequest {
            id: generate_pending_id(),
            path: path.to_string_lossy().into_owned(),
            operation: match access {
                Access::Read => PathOperation::Read,
                Access::Write => PathOperation::Write,
            },
            cwd: cwd.to_string_lossy().into_owned(),
            script_hash: script.clone(),
            run_in_background: self.run_in_background.then_some(true),
            session_id: self.ctx.session_id.clone(),
            created_at: now_iso(),
        };
        match write_pending_path(&self.ctx.host, &pending) {
            Ok(()) => messages::path_blocked(
                &pending.path,
                access == Access::Write,
                path.is_dir(),
                &pending.id,
            ),
            Err(e) => messages::denied(
                &pending.path,
                &[format!("cannot save pending approval: {e}")],
            ),
        }
    }
}

fn reason_text(decision: &Decision) -> Option<String> {
    match decision {
        Decision::Deny { reason, .. } => Some(
            match reason {
                Reason::Denied => "on a deny list",
                Reason::NotFound => "not found",
                Reason::ProjectDirWriteBlocked => "project dir writes are blocked",
                Reason::NotAllowed => "not allowed",
            }
            .to_string(),
        ),
        _ => None,
    }
}

impl Gate for PolicyGate {
    fn check_exec(&self, req: &ExecRequest<'_>) -> Result<(), String> {
        let program: PathBuf = if req.program.is_absolute() {
            req.program.to_path_buf()
        } else {
            req.cwd.join(req.program)
        };
        let problems = self
            .ctx
            .policy
            .check_exec(&req.argv, Some(&program), req.cwd);
        if problems.is_empty() {
            return Ok(());
        }
        let argv0 = req.argv.first().cloned().unwrap_or_default();
        if let Some(hard) = problems.iter().find(|p| p.is_hard()) {
            let (what, why) = match hard {
                Problem::Run { command, decision } => (command.clone(), reason_text(decision)),
                Problem::Path { path, decision, .. } => {
                    (path.to_string_lossy().into_owned(), reason_text(decision))
                }
                Problem::Rule { .. } => (argv0, Some("denied by a rule".into())),
                Problem::Ask(r) => (argv0, Some(r.clone())),
            };
            return Err(messages::denied(
                &what,
                &why.into_iter().collect::<Vec<_>>(),
            ));
        }
        // Only path problems: offer a path grant, as TS did for sandbox path errors.
        if let [Problem::Path { path, access, .. }] = problems.as_slice() {
            return Err(self.block_path(path, *access, req.cwd));
        }
        let mut commands = Vec::new();
        let mut reasons = Vec::new();
        for p in &problems {
            match p {
                Problem::Run { command, .. } => commands.push(command.clone()),
                Problem::Path { path, access, .. } => reasons.push(format!(
                    "{} {}",
                    if *access == Access::Write {
                        "writes"
                    } else {
                        "reads"
                    },
                    path.display()
                )),
                Problem::Rule { .. } => reasons.push("matches an ask rule".into()),
                Problem::Ask(r) => reasons.push(r.clone()),
            }
        }
        if commands.is_empty() {
            commands.push(argv0);
        }
        Err(self.block_command(&commands, req.cwd, &reasons))
    }

    fn runs_in_engine(&self, program: &Path, cwd: &Path) -> bool {
        self.ctx.policy.is_agent_script(program, cwd)
    }

    fn check_open(&self, path: &Path, access: Access) -> Result<(), String> {
        let cwd = path.parent().unwrap_or(path);
        match self.ctx.policy.check_path(path, cwd, access) {
            Decision::Allow { .. } => Ok(()),
            Decision::Ask { resolved, .. } => Err(self.block_path(&resolved, access, cwd)),
            d @ Decision::Deny { .. } => Err(messages::denied(
                &path.to_string_lossy(),
                &reason_text(&d).into_iter().collect::<Vec<_>>(),
            )),
        }
    }
}
