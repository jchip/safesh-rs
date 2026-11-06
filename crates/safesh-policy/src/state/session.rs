//! Session grant files: commands and path permissions allowed for one session.

use std::collections::BTreeSet;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::temp::session_file_path;
use super::{StateError, write_json_atomic};
use crate::host::HostEnv;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionPermissions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_commands: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permissions: Option<SessionPermissions>,
    /// Unknown top-level keys, kept across read-modify-write.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Reads the session file. A missing or corrupt file gives empty data.
#[must_use]
pub fn read_session_file(
    env: &HostEnv,
    project_dir: Option<&Path>,
    session_id: &str,
) -> SessionData {
    let Ok(path) = session_file_path(env, project_dir, session_id) else {
        return SessionData::default();
    };
    fs_err::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Shallow merge: fields present in `data` replace those on disk.
pub fn write_session_file(
    env: &HostEnv,
    data: SessionData,
    project_dir: Option<&Path>,
    session_id: &str,
) -> Result<(), StateError> {
    let path = session_file_path(env, project_dir, session_id)?;
    let mut merged = read_session_file(env, project_dir, session_id);
    if data.allowed_commands.is_some() {
        merged.allowed_commands = data.allowed_commands;
    }
    if data.permissions.is_some() {
        merged.permissions = data.permissions;
    }
    merged.extra.extend(data.extra);
    write_json_atomic(&path, &merged, true)
}

fn union(existing: Option<&Vec<String>>, add: &[String]) -> Vec<String> {
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut out = Vec::new();
    for s in existing.into_iter().flatten().chain(add) {
        if seen.insert(s) {
            out.push(s.clone());
        }
    }
    out
}

/// Adds commands, deduplicated, keeping first-seen order.
pub fn add_session_commands(
    env: &HostEnv,
    commands: &[String],
    project_dir: Option<&Path>,
    session_id: &str,
) -> Result<(), StateError> {
    let existing = read_session_file(env, project_dir, session_id);
    let merged = union(existing.allowed_commands.as_ref(), commands);
    let data = SessionData {
        allowed_commands: Some(merged),
        ..SessionData::default()
    };
    write_session_file(env, data, project_dir, session_id)
}

/// Adds read and write paths. An empty list leaves that side untouched, but a permissions object is always written.
pub fn add_session_paths(
    env: &HostEnv,
    read_paths: &[String],
    write_paths: &[String],
    project_dir: Option<&Path>,
    session_id: &str,
) -> Result<(), StateError> {
    let existing = read_session_file(env, project_dir, session_id);
    let mut permissions = existing.permissions.unwrap_or_default();
    if !read_paths.is_empty() {
        permissions.read = Some(union(permissions.read.as_ref(), read_paths));
    }
    if !write_paths.is_empty() {
        permissions.write = Some(union(permissions.write.as_ref(), write_paths));
    }
    let data = SessionData {
        permissions: Some(permissions),
        ..SessionData::default()
    };
    write_session_file(env, data, project_dir, session_id)
}

#[must_use]
pub fn get_session_allowed_commands(
    env: &HostEnv,
    project_dir: Option<&Path>,
    session_id: &str,
) -> BTreeSet<String> {
    get_session_allowed_commands_vec(env, project_dir, session_id)
        .into_iter()
        .collect()
}

#[must_use]
pub fn get_session_allowed_commands_vec(
    env: &HostEnv,
    project_dir: Option<&Path>,
    session_id: &str,
) -> Vec<String> {
    read_session_file(env, project_dir, session_id)
        .allowed_commands
        .unwrap_or_default()
}

#[must_use]
pub fn get_session_path_permissions(
    env: &HostEnv,
    project_dir: Option<&Path>,
    session_id: &str,
) -> SessionPermissions {
    read_session_file(env, project_dir, session_id)
        .permissions
        .unwrap_or_default()
}
