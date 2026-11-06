//! File locations and config loading.

use std::path::{Component, Path, PathBuf};

use serde::de::DeserializeOwned;

use super::defaults::default_config;
use super::merge::merge_configs;
use super::types::{Config, LocalConfig, extra_key_warnings};
use super::validate::validate_config;
use crate::host::HostEnv;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read {}: {source}", path.display())]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to write {}: {source}", path.display())]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("invalid config {}: {source}", path.display())]
    Parse {
        path: PathBuf,
        source: serde_path_to_error::Error<serde_json::Error>,
    },
    #[error("failed to serialize config: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("Config validation failed:\n{}", .0.join("\n"))]
    Validation(Vec<String>),
}

/// Merged config and the warnings gathered while loading.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedConfig {
    pub config: Config,
    pub warnings: Vec<String>,
}

/// Join and drop `.` parts, resolve `..` lexically.
fn join_normalized(base: &Path, parts: &[&str]) -> PathBuf {
    let mut out = PathBuf::new();
    let full = parts.iter().fold(base.to_path_buf(), |p, s| p.join(s));
    for comp in full.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// `$XDG_CONFIG_HOME/safesh`, else `~/.config/safesh`. Same on macOS.
#[must_use]
pub fn get_global_config_dir(env: &HostEnv) -> PathBuf {
    match &env.xdg_config_home {
        Some(xdg) if !xdg.as_os_str().is_empty() => join_normalized(xdg, &["safesh"]),
        _ => join_normalized(&env.home, &[".config", "safesh"]),
    }
}

#[must_use]
pub fn get_global_config_json_path(env: &HostEnv) -> PathBuf {
    get_global_config_dir(env).join("config.json")
}

#[must_use]
pub fn get_project_config_dir(project_dir: &Path) -> PathBuf {
    join_normalized(project_dir, &[".config", "safesh"])
}

#[must_use]
pub fn get_project_config_json_path(project_dir: &Path) -> PathBuf {
    get_project_config_dir(project_dir).join("config.json")
}

#[must_use]
pub fn get_local_json_config_path(project_dir: &Path) -> PathBuf {
    get_project_config_dir(project_dir).join("config.local.json")
}

fn exists(path: &Path) -> bool {
    fs_err::metadata(path).is_ok()
}

/// Read a file. `None` when it does not exist.
pub(super) fn read_optional(path: &Path) -> Result<Option<String>, ConfigError> {
    match fs_err::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(ConfigError::Read {
            path: path.to_path_buf(),
            source,
        }),
    }
}

pub(super) fn parse_json<T: DeserializeOwned>(path: &Path, text: &str) -> Result<T, ConfigError> {
    let mut de = serde_json::Deserializer::from_str(text);
    serde_path_to_error::deserialize(&mut de).map_err(|source| ConfigError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

/// Load `config.json` at `path`, if present.
pub(super) fn load_json_config(path: &Path) -> Result<Option<Config>, ConfigError> {
    match read_optional(path)? {
        Some(text) => parse_json(path, &text).map(Some),
        None => Ok(None),
    }
}

fn ts_config_warning(dir: &Path, stem: &str) -> Option<String> {
    let ts = dir.join(format!("{stem}.ts"));
    let json = dir.join(format!("{stem}.json"));
    (exists(&ts) && !exists(&json)).then(|| {
        format!(
            "{} is not loaded (TypeScript config is no longer supported); run `desh migrate-config`",
            ts.display()
        )
    })
}

/// Load default, global, project and local config, merged in that order.
///
/// Project files live under `project_dir`, or `cwd` when it is `None`. Paths inside the
/// config stay unexpanded. Validation errors fail the load; warnings are returned.
pub fn load_config(
    cwd: &Path,
    project_dir: Option<&Path>,
    env: &HostEnv,
) -> Result<LoadedConfig, ConfigError> {
    let mut warnings = Vec::new();
    let mut config = default_config();

    let global_dir = get_global_config_dir(env);
    let project_root = project_dir.unwrap_or(cwd);
    let project_dir = get_project_config_dir(project_root);

    for (dir, stem) in [
        (&global_dir, "config"),
        (&project_dir, "config"),
        (&project_dir, "config.local"),
    ] {
        warnings.extend(ts_config_warning(dir, stem));
    }

    for path in [
        get_global_config_json_path(env),
        get_project_config_json_path(project_root),
    ] {
        if let Some(loaded) = load_json_config(&path)? {
            warnings.extend(extra_key_warnings(
                &path.display().to_string(),
                &loaded.extra,
            ));
            config = merge_configs(&config, &loaded);
        }
    }

    // A broken local file only warns.
    let local_path = get_local_json_config_path(project_root);
    let label = local_path.display().to_string();
    match read_optional(&local_path).map(|t| t.map(|t| parse_json::<LocalConfig>(&local_path, &t)))
    {
        Ok(Some(Ok(local))) => {
            warnings.extend(extra_key_warnings(&label, &local.extra));
            let (converted, more) = local.to_config(&label);
            warnings.extend(more);
            if let Some(converted) = converted {
                config = merge_configs(&config, &converted);
            }
        }
        Ok(Some(Err(e))) | Err(e) => warnings.push(format!("failed to load local config: {e}")),
        Ok(None) => {}
    }

    if let Some(dir) = config.workspace_dir.clone() {
        config.workspace.get_or_insert(dir);
    }

    let validation = validate_config(&config);
    if !validation.errors.is_empty() {
        return Err(ConfigError::Validation(validation.errors));
    }
    warnings.extend(validation.warnings);
    Ok(LoadedConfig { config, warnings })
}
