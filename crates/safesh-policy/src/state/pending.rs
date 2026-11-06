//! Pending approval files for blocked commands and paths.

use serde::{Deserialize, Serialize};

use super::temp::{generate_temp_id, pending_file_path, pending_path_file_path};
use super::{StateError, write_json_atomic};
use crate::host::HostEnv;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingCommand {
    pub id: String,
    pub script_hash: String,
    pub commands: Vec<String>,
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_in_background: Option<bool>,
    /// Agent session id, so retry choice "this session" knows where to write (D3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// ISO 8601.
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PathOperation {
    Read,
    Write,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingPathRequest {
    pub id: String,
    pub path: String,
    pub operation: PathOperation,
    pub cwd: String,
    pub script_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_in_background: Option<bool>,
    /// Agent session id, so retry choice "this session" knows where to write (D3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// ISO 8601.
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingKind {
    Command,
    Path,
}

/// `<unix millis>-<pid>`.
#[must_use]
pub fn generate_pending_id() -> String {
    generate_temp_id()
}

pub fn write_pending_command(env: &HostEnv, pending: &PendingCommand) -> Result<(), StateError> {
    let path = pending_file_path(env, &pending.id)?;
    write_json_atomic(&path, pending, false)
}

pub fn write_pending_path(env: &HostEnv, pending: &PendingPathRequest) -> Result<(), StateError> {
    let path = pending_path_file_path(env, &pending.id)?;
    write_json_atomic(&path, pending, false)
}

/// None if the file is missing or corrupt.
#[must_use]
pub fn read_pending_command(env: &HostEnv, id: &str) -> Option<PendingCommand> {
    let path = pending_file_path(env, id).ok()?;
    serde_json::from_str(&fs_err::read_to_string(path).ok()?).ok()
}

/// None if the file is missing or corrupt.
#[must_use]
pub fn read_pending_path(env: &HostEnv, id: &str) -> Option<PendingPathRequest> {
    let path = pending_path_file_path(env, id).ok()?;
    serde_json::from_str(&fs_err::read_to_string(path).ok()?).ok()
}

/// Removes the file; a missing file is not an error.
pub fn delete_pending(env: &HostEnv, id: &str, kind: PendingKind) {
    let path = match kind {
        PendingKind::Command => pending_file_path(env, id),
        PendingKind::Path => pending_path_file_path(env, id),
    };
    if let Ok(p) = path {
        let _ = fs_err::remove_file(p);
    }
}
