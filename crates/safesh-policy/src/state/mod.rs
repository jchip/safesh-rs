//! Session grant files and pending approval files.

pub mod pending;
pub mod session;
pub mod temp;

use std::path::Path;

use serde::Serialize;
use std::io::Write;

#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("cannot persist file: {0}")]
    Persist(#[from] tempfile::PersistError),
}

/// Writes `value` as pretty JSON atomically: temp file in the target dir, then rename.
pub(crate) fn write_json_atomic<T: Serialize>(
    path: &Path,
    value: &T,
    trailing_newline: bool,
) -> Result<(), StateError> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    fs_err::create_dir_all(dir)?;
    let mut content = serde_json::to_string_pretty(value)?;
    if trailing_newline {
        content.push('\n');
    }
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    tmp.write_all(content.as_bytes())?;
    tmp.persist(path)?;
    Ok(())
}
