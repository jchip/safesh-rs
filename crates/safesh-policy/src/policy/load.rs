//! Config for one agent session: merged config files, project root, git roots and session grants.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::config::{Config, ConfigError, load_config, merge_configs};
use crate::host::HostEnv;
use crate::project::{
    FindProjectRootOptions, expand_git_workspace_roots, find_git_workspace_roots, find_project_root,
};
use crate::state::session::read_session_file;

#[derive(Debug, Clone)]
pub struct LoadedSession {
    pub config: Config,
    pub project_dir: PathBuf,
    pub warnings: Vec<String>,
}

/// Port of TS `loadSessionConfig`. Session grants are keyed by the hook's `session_id` (D3).
/// Project config files are read from `cwd`, as TS does.
pub fn load_session_config(
    cwd: &Path,
    session_id: Option<&str>,
    host: &HostEnv,
) -> Result<LoadedSession, ConfigError> {
    let project_dir = find_project_root(cwd, FindProjectRootOptions::default(), host);
    let git_roots = expand_git_workspace_roots(&find_git_workspace_roots(cwd, host), host);
    let loaded = load_config(cwd, None, host)?;
    let overlay = Config {
        project_dir: Some(project_dir.to_string_lossy().into_owned()),
        workspace_roots: Some(git_roots),
        ..Config::default()
    };
    let mut config = merge_configs(&loaded.config, &overlay);
    config.workspace_roots = Some(expand_git_workspace_roots(
        config.workspace_roots.as_deref().unwrap_or_default(),
        host,
    ));
    if let Some(sid) = session_id {
        merge_session_permissions(&mut config, &project_dir, sid, host);
    }
    Ok(LoadedSession {
        config,
        project_dir,
        warnings: loaded.warnings,
    })
}

/// Port of TS `mergeSessionPermissions`: adds session read, write and command grants, deduped.
pub fn merge_session_permissions(
    config: &mut Config,
    project_dir: &Path,
    session_id: &str,
    host: &HostEnv,
) {
    let session = read_session_file(host, Some(project_dir), session_id);
    let sp = session.permissions.unwrap_or_default();
    let perms = config.permissions.get_or_insert_with(Default::default);
    union(&mut perms.read, sp.read);
    union(&mut perms.write, sp.write);
    union(&mut perms.run, session.allowed_commands);
}

fn union(target: &mut Option<Vec<String>>, extra: Option<Vec<String>>) {
    let Some(extra) = extra else { return };
    let list = target.get_or_insert_with(Vec::new);
    let mut seen: BTreeSet<String> = list.iter().cloned().collect();
    list.extend(extra.into_iter().filter(|s| seen.insert(s.clone())));
}
