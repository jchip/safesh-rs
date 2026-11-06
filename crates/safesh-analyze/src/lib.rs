//! Static pre-check: walks the brush-parser AST and lists what a script would run and open.
//!
//! The walk resolves what it can and marks the rest [`Word::Unknown`]. Unknown parts are left
//! to the runtime gates in `safesh run`, so the pre-check never denies on its own.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

mod builtins;
mod control;
mod path;
mod scan;
mod walk;
mod words;

pub use safesh_policy::effect::Access;

#[derive(Debug, thiserror::Error)]
pub enum AnalyzeError {
    #[error("parse error: {0}")]
    Parse(String),
}

/// Inputs from the hook.
#[derive(Debug, Clone, Default)]
pub struct AnalyzeOptions {
    /// Absolute cwd the script starts in.
    pub base_cwd: PathBuf,
    /// Env values visible to static expansion, such as `HOME`. Script assignments shadow them.
    pub env: BTreeMap<String, String>,
}

/// A word after static expansion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Word {
    /// Fully known value, quotes removed.
    Known(String),
    /// Depends on something the walk can't know, such as an unset variable, `$(...)` output or a glob.
    /// Holds the raw source text.
    Unknown(String),
}

/// Static working directory at some point in the script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cwd {
    /// Absolute path.
    Known(PathBuf),
    /// A `cd` went somewhere the walk can't follow.
    Unknown,
}

/// A simple command the script would run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exec {
    /// `argv[0]` is the command name as written, after variable resolution.
    pub argv: Vec<Word>,
    pub cwd: Cwd,
    /// A bash builtin run in-process (`echo`, `cd`, `test`...). No exec happens.
    pub builtin: bool,
    /// A call to a function defined in the script.
    pub function: bool,
}

/// Why a path would be opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenKind {
    /// `>`, `>>`, `<>`, `<`, `&>` and friends.
    Redirect,
    /// `source` or `.`.
    Source,
}

/// A file the script would open from the shell itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Open {
    pub path: Word,
    pub access: Access,
    pub kind: OpenKind,
    pub cwd: Cwd,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Analysis {
    /// Every simple command in source order, including inside `$(...)`, `<(...)`, functions and branches.
    pub execs: Vec<Exec>,
    /// Every redirect and `source` target, excluding device paths like `/dev/null`.
    pub opens: Vec<Open>,
    /// Functions the script defines.
    pub functions: BTreeSet<String>,
    /// Cwd after every `&&` step succeeds. `None` when unknown.
    pub final_cwd: Option<PathBuf>,
    /// True when the script may change cwd (`cd`, `pushd`, `popd`, `source`, `eval`, or an unknown command name).
    pub may_change_cwd: bool,
}

/// Walk `script` and list what it would run and open. Nothing is executed.
///
/// Only a parse error fails. Constructs the walk can't follow become `Unknown` values.
pub fn analyze(script: &str, opts: &AnalyzeOptions) -> Result<Analysis, AnalyzeError> {
    let mut w = walk::Walker::new(opts);
    let prog =
        walk::parse_script(script, &w.popts).map_err(|e| AnalyzeError::Parse(e.to_string()))?;
    w.program(&prog);
    Ok(w.finish())
}

/// True when `script` contains a control-plane call (`safesh run --id`, `safesh retry`,
/// `safesh retry-path`, or the same under `desh` for retry), possibly behind leading
/// assignments, `&&`, `||`, `|`, `;`, a subshell or a brace group.
#[must_use]
pub fn is_control_plane(script: &str) -> bool {
    control::is_control_plane(script)
}
