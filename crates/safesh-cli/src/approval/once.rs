//! Once-grants: what the user approved for exactly one run of one script.
//!
//! The hook writes `<state>/once/<pending id>.json`. `safesh run --pending <id>` claims it by
//! rename, so a second run finds nothing. `<state>` is on the deny-write list.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use safesh_policy::config::Config;
use safesh_policy::state::temp::check_id;
use serde::{Deserialize, Serialize};

/// A once-grant is used right after the hook writes it. Older ones are refused.
const MAX_AGE_MS: u64 = 5 * 60 * 1000;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnceGrant {
    pub script_id: String,
    #[serde(default)]
    pub commands: Vec<String>,
    #[serde(default)]
    pub read: Vec<String>,
    #[serde(default)]
    pub write: Vec<String>,
    #[serde(default)]
    pub created_ms: u64,
}

impl OnceGrant {
    /// Adds the grant to the config for this run.
    pub fn apply(self, cfg: &mut Config) {
        let p = cfg.permissions.get_or_insert_with(Default::default);
        p.run.get_or_insert_with(Vec::new).extend(self.commands);
        p.read.get_or_insert_with(Vec::new).extend(self.read);
        p.write.get_or_insert_with(Vec::new).extend(self.write);
    }
}

fn dir(state_dir: &Path) -> PathBuf {
    state_dir.join("once")
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

pub fn write(state_dir: &Path, pending_id: &str, grant: &OnceGrant) -> Result<(), String> {
    check_id(pending_id).map_err(|e| e.to_string())?;
    let d = dir(state_dir);
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&d)
        .map_err(|e| e.to_string())?;
    let mut g = grant.clone();
    g.created_ms = now_ms();
    let body = serde_json::to_vec(&g).map_err(|e| e.to_string())?;
    let path = d.join(format!("{pending_id}.json"));
    let _ = fs::remove_file(&path);
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .and_then(|mut f| f.write_all(&body))
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// Takes the grant for `pending_id` if it exists, is fresh, and is for `script_id`.
/// The file is gone afterwards in every case.
pub fn claim(state_dir: &Path, pending_id: &str, script_id: &str) -> Option<OnceGrant> {
    check_id(pending_id).ok()?;
    let d = dir(state_dir);
    let claimed = d.join(format!(".claimed-{pending_id}-{}", std::process::id()));
    fs::rename(d.join(format!("{pending_id}.json")), &claimed).ok()?;
    let g: Option<OnceGrant> = fs::read(&claimed)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok());
    let _ = fs::remove_file(&claimed);
    g.filter(|g| g.script_id == script_id && now_ms().saturating_sub(g.created_ms) <= MAX_AGE_MS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grant(script: &str) -> OnceGrant {
        OnceGrant {
            script_id: script.into(),
            commands: vec!["foo".into()],
            ..OnceGrant::default()
        }
    }

    #[test]
    fn claim_once_only() {
        let t = tempfile::tempdir().unwrap();
        write(t.path(), "p1", &grant("s1")).unwrap();
        assert_eq!(claim(t.path(), "p1", "s1").unwrap().commands, vec!["foo"]);
        assert!(claim(t.path(), "p1", "s1").is_none());
    }

    #[test]
    fn wrong_script_is_refused_and_removed() {
        let t = tempfile::tempdir().unwrap();
        write(t.path(), "p1", &grant("s1")).unwrap();
        assert!(claim(t.path(), "p1", "other").is_none());
        assert!(claim(t.path(), "p1", "s1").is_none());
    }

    #[test]
    fn stale_is_refused() {
        let t = tempfile::tempdir().unwrap();
        write(t.path(), "p1", &grant("s1")).unwrap();
        let path = dir(t.path()).join("p1.json");
        let mut g: OnceGrant = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        g.created_ms -= MAX_AGE_MS + 1;
        fs::write(&path, serde_json::to_vec(&g).unwrap()).unwrap();
        assert!(claim(t.path(), "p1", "s1").is_none());
    }

    #[test]
    fn bad_id_is_refused() {
        let t = tempfile::tempdir().unwrap();
        assert!(write(t.path(), "../x", &grant("s1")).is_err());
        assert!(claim(t.path(), "../x", "s1").is_none());
    }
}
