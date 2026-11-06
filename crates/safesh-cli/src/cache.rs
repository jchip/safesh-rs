//! Script cache: the hook stores an approved script, `safesh run --id` loads it.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::PathBuf;

use safesh_policy::host::HostEnv;
use sha2::{Digest, Sha256};

/// Bump when the meaning of a cached script changes.
const CACHE_VERSION: &str = "1";

/// Content id of a script: first 16 hex chars of sha256(version, script).
pub fn script_id(script: &str) -> String {
    let mut h = Sha256::new();
    h.update(CACHE_VERSION.as_bytes());
    h.update([0]);
    h.update(script.as_bytes());
    let digest = h.finalize();
    let mut out = String::with_capacity(16);
    for b in digest.iter().take(8) {
        let _ = write!(out, "{b:02x}");
    }
    out
}

fn script_path(host: &HostEnv, id: &str) -> Result<PathBuf, String> {
    safesh_policy::state::temp::check_id(id).map_err(|e| e.to_string())?;
    Ok(host
        .state_dir
        .join("scripts")
        .join(format!("script-{id}.sh")))
}

/// Stores `script` and returns its id.
pub fn store(host: &HostEnv, script: &str) -> Result<String, String> {
    let id = script_id(script);
    let path = script_path(host, &id)?;
    let dir = path.parent().ok_or("bad cache path")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut tmp = tempfile_in(dir)?;
    tmp.1
        .write_all(script.as_bytes())
        .map_err(|e| e.to_string())?;
    std::fs::rename(&tmp.0, &path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(id)
}

pub fn load(host: &HostEnv, id: &str) -> Result<String, String> {
    let path = script_path(host, id)?;
    std::fs::read_to_string(&path).map_err(|e| format!("script {id} not found: {e}"))
}

/// A uniquely named temp file in `dir`.
fn tempfile_in(dir: &std::path::Path) -> Result<(PathBuf, std::fs::File), String> {
    for n in 0..100u32 {
        let p = dir.join(format!(".tmp-{}-{n}", std::process::id()));
        if let Ok(f) = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&p)
        {
            return Ok((p, f));
        }
    }
    Err(format!("cannot create a temp file in {}", dir.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_is_stable_and_short() {
        assert_eq!(script_id("ls"), script_id("ls"));
        assert_ne!(script_id("ls"), script_id("ls "));
        assert_eq!(script_id("ls").len(), 16);
    }
}
