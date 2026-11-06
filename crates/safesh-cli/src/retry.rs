//! `safesh retry` and `safesh retry-path`: apply the user's choice, then run the cached script.

use std::path::{Path, PathBuf};

use safesh_policy::config::{add_commands_to_config, add_paths_to_config, save_to_global_json};
use safesh_policy::host::HostEnv;
use safesh_policy::state::pending::{
    PathOperation, PendingKind, delete_pending, read_pending_command, read_pending_path,
};
use safesh_policy::state::session::{add_session_commands, add_session_paths};

use crate::context::Context;
use crate::{cache, command_args, execute, fail};

/// Where a grant lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Once,
    Session,
    Project,
    Global,
}

/// Runs the cached script in `cwd` with `extra` grants applied for this run only.
fn rerun(
    host: HostEnv,
    cwd: &Path,
    session: Option<&str>,
    script_id: &str,
    background: bool,
    grant: impl FnOnce(&mut safesh_policy::config::Config),
) -> u8 {
    let script = match cache::load(&host, script_id) {
        Ok(s) => s,
        Err(e) => return fail(&e),
    };
    if let Err(e) = std::env::set_current_dir(cwd) {
        return fail(&format!("{}: {e}", cwd.display()));
    }
    let ctx = match Context::load_granted(host, cwd, session, grant) {
        Ok(c) => c,
        Err(e) => return fail(&e),
    };
    execute(
        ctx,
        command_args(script),
        Some(script_id.to_string()),
        background,
    )
}

/// Saves a command grant for `scope`. Returns the scope used: a session grant without a
/// session id becomes once.
pub fn save_command_grant(
    host: &HostEnv,
    commands: &[String],
    cwd: &Path,
    project: &Path,
    session: Option<&str>,
    scope: Scope,
) -> Result<Scope, String> {
    match (scope, session) {
        (Scope::Project, _) => add_commands_to_config(commands, cwd).map_err(|e| e.to_string())?,
        (Scope::Global, _) => save_to_global_json(host, commands).map_err(|e| e.to_string())?,
        (Scope::Session, Some(sid)) => {
            add_session_commands(host, commands, Some(project), sid).map_err(|e| e.to_string())?;
        }
        (Scope::Session, None) => {
            eprintln!("safesh: no session id; allowing once instead");
            return Ok(Scope::Once);
        }
        (Scope::Once, _) => {}
    }
    Ok(scope)
}

/// Path version of [`save_command_grant`]. Paths have no global scope; it is saved as once.
pub fn save_path_grant(
    host: &HostEnv,
    read: &[String],
    write: &[String],
    cwd: &Path,
    session: Option<&str>,
    scope: Scope,
) -> Result<Scope, String> {
    match (scope, session) {
        (Scope::Project, _) => add_paths_to_config(read, write, cwd).map_err(|e| e.to_string())?,
        (Scope::Session, Some(sid)) => {
            add_session_paths(host, read, write, Some(cwd), sid).map_err(|e| e.to_string())?;
        }
        (Scope::Session, None) => {
            eprintln!("safesh: no session id; allowing once instead");
            return Ok(Scope::Once);
        }
        (Scope::Global | Scope::Once, _) => return Ok(Scope::Once),
    }
    Ok(scope)
}

pub fn retry(id: &str, choice: &str) -> u8 {
    let host = HostEnv::from_process();
    let Some(p) = read_pending_command(&host, id) else {
        return fail(&format!("no pending approval {id}; it may have expired"));
    };
    let scope = match choice {
        "1" => Scope::Once,
        "2" => Scope::Project,
        "3" => Scope::Session,
        "5" => Scope::Global,
        "4" => {
            eprintln!("[safesh] Command denied by user.");
            delete_pending(&host, id, PendingKind::Command);
            return 0;
        }
        _ => return fail("--choice must be 1-5"),
    };
    let cwd = PathBuf::from(&p.cwd);
    let project = p
        .project_dir
        .as_ref()
        .map_or_else(|| cwd.clone(), PathBuf::from);
    let session = p.session_id.as_deref();
    let saved = save_command_grant(&host, &p.commands, &cwd, &project, session, scope);
    if let Err(e) = saved {
        return fail(&e);
    }
    delete_pending(&host, id, PendingKind::Command);
    let commands = p.commands.clone();
    rerun(
        host,
        &cwd,
        session,
        &p.script_hash,
        p.run_in_background == Some(true),
        // Every scope also grants for this run, so the rerun can't block on the same command.
        move |c| {
            let run = c
                .permissions
                .get_or_insert_with(Default::default)
                .run
                .get_or_insert_with(Vec::new);
            run.extend(commands);
        },
    )
}

/// Parsed `--choice` for retry-path: `<r|w|rw><1|2|3>[d]`.
#[derive(Debug, PartialEq, Eq)]
struct PathChoice {
    read: bool,
    write: bool,
    scope: Scope,
    dir: bool,
}

fn parse_path_choice(choice: &str) -> Option<PathChoice> {
    let (rw, rest) = if let Some(r) = choice.strip_prefix("rw") {
        ((true, true), r)
    } else if let Some(r) = choice.strip_prefix('r') {
        ((true, false), r)
    } else if let Some(r) = choice.strip_prefix('w') {
        ((false, true), r)
    } else {
        return None;
    };
    let (scope, rest) = match rest.split_at_checked(1)? {
        ("1", r) => (Scope::Once, r),
        ("2", r) => (Scope::Session, r),
        ("3", r) => (Scope::Project, r),
        _ => return None,
    };
    let dir = match rest {
        "" => false,
        "d" => true,
        _ => return None,
    };
    Some(PathChoice {
        read: rw.0,
        write: rw.1,
        scope,
        dir,
    })
}

pub fn retry_path(id: &str, choice: &str) -> u8 {
    let host = HostEnv::from_process();
    let Some(p) = read_pending_path(&host, id) else {
        return fail(&format!(
            "no pending path approval {id}; it may have expired"
        ));
    };
    if choice == "4" || choice == "deny" {
        eprintln!("[safesh] Path access denied by user.");
        delete_pending(&host, id, PendingKind::Path);
        return 0;
    }
    let Some(c) = parse_path_choice(choice) else {
        return fail("--choice must look like r1, w2, rw3, w2d, or 4 to deny");
    };
    let path = PathBuf::from(&p.path);
    let target = if c.dir {
        if path.is_dir() {
            path.clone()
        } else {
            path.parent()
                .map_or_else(|| path.clone(), Path::to_path_buf)
        }
    } else {
        path.clone()
    };
    let target = target.to_string_lossy().into_owned();
    let read: Vec<String> = if c.read || p.operation == PathOperation::Read && !c.write {
        vec![target.clone()]
    } else {
        Vec::new()
    };
    let write: Vec<String> = if c.write {
        vec![target.clone()]
    } else {
        Vec::new()
    };
    let cwd = PathBuf::from(&p.cwd);
    let session = p.session_id.as_deref();
    let saved = save_path_grant(&host, &read, &write, &cwd, session, c.scope);
    if let Err(e) = saved {
        return fail(&e);
    }
    rerun(
        host,
        &cwd,
        session,
        &p.script_hash,
        p.run_in_background == Some(true),
        move |cfg| {
            let perms = cfg.permissions.get_or_insert_with(Default::default);
            perms.read.get_or_insert_with(Vec::new).extend(read);
            perms.write.get_or_insert_with(Vec::new).extend(write);
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_choices() {
        let c = parse_path_choice("rw3d").unwrap();
        assert!(c.read && c.write && c.dir && c.scope == Scope::Project);
        let c = parse_path_choice("w2").unwrap();
        assert!(!c.read && c.write && !c.dir && c.scope == Scope::Session);
        assert!(parse_path_choice("x1").is_none());
        assert!(parse_path_choice("r4").is_none());
        assert!(parse_path_choice("r1x").is_none());
    }
}
