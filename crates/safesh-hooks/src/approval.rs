//! Single-use records that tie a Codex `PreToolUse` rewrite to its `PermissionRequest`.

use fs_err::os::unix::fs::OpenOptionsExt;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use safesh_policy::host::HostEnv;
use serde::{Deserialize, Serialize};

const MAX_AGE_MS: u64 = 60_000;

/// Fields a `PermissionRequest` must match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRequest {
    pub session_id: String,
    pub turn_id: String,
    pub cwd: String,
    pub command: String,
}

/// On-disk record, `approval-<id>.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRecord {
    pub session_id: String,
    pub turn_id: String,
    pub cwd: String,
    pub command: String,
    /// Milliseconds since the Unix epoch.
    pub created_at: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum ApprovalError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

fn dir(host: &HostEnv) -> Result<PathBuf, ApprovalError> {
    let d = host.state_dir.join("approvals");
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&d)?;
    fs_err::set_permissions(&d, std::fs::Permissions::from_mode(0o700))?;
    Ok(d)
}

fn expired(r: &ApprovalRecord, now: u64) -> bool {
    r.created_at > now + MAX_AGE_MS || now.saturating_sub(r.created_at) > MAX_AGE_MS
}

fn read_record(path: &Path) -> Option<ApprovalRecord> {
    let meta = fs_err::symlink_metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    serde_json::from_str(&fs_err::read_to_string(path).ok()?).ok()
}

fn is_record_name(name: &str) -> bool {
    name.starts_with("approval-")
        && std::path::Path::new(name)
            .extension()
            .is_some_and(|e| e == "json")
}

fn unique() -> String {
    static N: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!(
        "{}-{nanos}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    )
}

fn cleanup(d: &Path, now: u64) -> Result<(), ApprovalError> {
    for entry in fs_err::read_dir(d)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str().filter(|n| n.starts_with("approval-")) else {
            continue;
        };
        let path = d.join(name);
        if read_record(&path).is_none_or(|r| expired(&r, now)) {
            let _ = fs_err::remove_file(&path);
        }
    }
    Ok(())
}

/// Record an approval for a rewritten command. `now_ms` is the current time in milliseconds.
pub fn write_approval(
    host: &HostEnv,
    request: &ApprovalRequest,
    now_ms: u64,
) -> Result<(), ApprovalError> {
    let d = dir(host)?;
    cleanup(&d, now_ms)?;
    let record = ApprovalRecord {
        session_id: request.session_id.clone(),
        turn_id: request.turn_id.clone(),
        cwd: request.cwd.clone(),
        command: request.command.clone(),
        created_at: now_ms,
    };
    let path = d.join(format!("approval-{}.json", unique()));
    let mut f = fs_err::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(serde_json::to_string(&record)?.as_bytes())?;
    Ok(())
}

/// Claim a matching, unexpired record. Returns `true` for exactly one caller per record.
pub fn consume_approval(
    host: &HostEnv,
    request: &ApprovalRequest,
    now_ms: u64,
) -> Result<bool, ApprovalError> {
    let d = dir(host)?;
    for entry in fs_err::read_dir(&d)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str().filter(|n| is_record_name(n)) else {
            continue;
        };
        let path = d.join(name);
        let Some(r) = read_record(&path) else {
            let _ = fs_err::remove_file(&path);
            continue;
        };
        if expired(&r, now_ms) {
            let _ = fs_err::remove_file(&path);
            continue;
        }
        if r.session_id != request.session_id
            || r.turn_id != request.turn_id
            || r.cwd != request.cwd
            || r.command != request.command
        {
            continue;
        }
        let claimed = d.join(format!("{name}.claimed-{}", unique()));
        if fs_err::rename(&path, &claimed).is_err() {
            continue;
        }
        let _ = fs_err::remove_file(&claimed);
        return Ok(true);
    }
    Ok(false)
}

/// Stdout for a `PermissionRequest` hook that approves the call.
#[must_use]
pub fn format_permission_request_allow() -> String {
    "{\"hookSpecificOutput\":{\"hookEventName\":\"PermissionRequest\",\"decision\":{\"behavior\":\"allow\"}}}\n"
        .to_string()
}
