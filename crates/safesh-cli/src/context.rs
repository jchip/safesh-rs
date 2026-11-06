//! Per-invocation context: host env, merged config and the policy built from it.

use std::path::{Path, PathBuf};

use safesh_policy::config::Config;
use safesh_policy::host::HostEnv;
use safesh_policy::policy::{Policy, PolicyOptions, load_session_config};

/// Agent session id, carried into `safesh run` and nested shells.
pub const ENV_SESSION: &str = "SAFESH_SESSION_ID";
/// Cwd the hook saw. Fills `${CWD}` and anchors config lookup in nested shells.
pub const ENV_BASE_CWD: &str = "SAFESH_BASE_CWD";
/// Cached script id, so a runtime denial can point retry at it.
pub const ENV_SCRIPT: &str = "SAFESH_SCRIPT_ID";

pub struct Context {
    pub host: HostEnv,
    pub base_cwd: PathBuf,
    pub session_id: Option<String>,
    pub project_dir: PathBuf,
    pub policy: Policy,
    pub warnings: Vec<String>,
    pub approval: safesh_policy::config::ResolvedApproval,
}

impl Context {
    pub fn load(base_cwd: &Path, session_id: Option<&str>) -> Result<Self, String> {
        let host = HostEnv::from_process();
        Self::load_with(host, base_cwd, session_id)
    }

    pub fn load_with(
        host: HostEnv,
        base_cwd: &Path,
        session_id: Option<&str>,
    ) -> Result<Self, String> {
        Self::load_granted(host, base_cwd, session_id, |_| {})
    }

    /// Like [`Self::load_with`], with `grant` applied to the merged config before the policy is
    /// built. Used for one-time approvals.
    pub fn load_granted(
        host: HostEnv,
        base_cwd: &Path,
        session_id: Option<&str>,
        grant: impl FnOnce(&mut Config),
    ) -> Result<Self, String> {
        let mut loaded =
            load_session_config(base_cwd, session_id, &host).map_err(|e| e.to_string())?;
        grant(&mut loaded.config);
        let approval =
            safesh_policy::config::ApprovalConfig::resolved(loaded.config.approval.as_ref());
        let opts = PolicyOptions {
            session_id: session_id.map(str::to_string),
        };
        let policy =
            Policy::new(&loaded.config, base_cwd, &host, &opts).map_err(|e| e.to_string())?;
        Ok(Self {
            host,
            base_cwd: base_cwd.to_path_buf(),
            session_id: session_id.map(str::to_string),
            project_dir: loaded.project_dir,
            policy,
            warnings: loaded.warnings,
            approval,
        })
    }

    /// Rebuilds the context a parent `safesh run` passed down through env.
    pub fn from_env() -> Result<Self, String> {
        let base = std::env::var_os(ENV_BASE_CWD)
            .map(PathBuf::from)
            .map_or_else(std::env::current_dir, Ok)
            .map_err(|e| e.to_string())?;
        let session = std::env::var(ENV_SESSION).ok().filter(|s| !s.is_empty());
        Self::load(&base, session.as_deref())
    }

    /// Env vars that let nested shells rebuild this context.
    pub fn nested_env(&self, script_id: Option<&str>) -> Vec<(String, String)> {
        let mut env = vec![(
            ENV_BASE_CWD.to_string(),
            self.base_cwd.to_string_lossy().into_owned(),
        )];
        if let Some(s) = &self.session_id {
            env.push((ENV_SESSION.to_string(), s.clone()));
        }
        if let Some(id) = script_id {
            env.push((ENV_SCRIPT.to_string(), id.to_string()));
        }
        env
    }
}
