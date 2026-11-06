//! State file locations and id generation.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::StateError;
use crate::host::HostEnv;

/// `<state_dir>/pending`, created if missing.
pub fn pending_dir(env: &HostEnv) -> Result<PathBuf, StateError> {
    let dir = env.state_dir.join("pending");
    fs_err::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn pending_file_path(env: &HostEnv, id: &str) -> Result<PathBuf, StateError> {
    check_id(id)?;
    Ok(pending_dir(env)?.join(format!("pending-{id}.json")))
}

pub fn pending_path_file_path(env: &HostEnv, id: &str) -> Result<PathBuf, StateError> {
    check_id(id)?;
    Ok(pending_dir(env)?.join(format!("pending-path-{id}.json")))
}

/// `<project>/.temp/safesh/session-<id>.json`, or `<state_dir>/session-<id>.json` without a project dir.
/// The parent directory is created if missing.
pub fn session_file_path(
    env: &HostEnv,
    project_dir: Option<&Path>,
    session_id: &str,
) -> Result<PathBuf, StateError> {
    check_id(session_id)?;
    let dir = match project_dir {
        Some(p) => p.join(".temp").join("safesh"),
        None => env.state_dir.clone(),
    };
    fs_err::create_dir_all(&dir)?;
    Ok(dir.join(format!("session-{session_id}.json")))
}

/// `<unix millis>-<pid>`.
#[must_use]
pub fn generate_temp_id() -> String {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    format!("{ms}-{}", std::process::id())
}

/// Rejects ids that are empty or could escape the state dir, such as `../x`.
pub fn check_id(id: &str) -> Result<(), StateError> {
    let ok = !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if ok {
        Ok(())
    } else {
        Err(StateError::InvalidId(id.to_string()))
    }
}
