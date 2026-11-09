//! Config shapes. Every field is optional so merge can tell unset from set.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Top-level keys that older configs may carry. They are accepted, ignored and warned about.
pub const DROPPED_KEYS: &[&str] = &[
    "imports",
    "denoFlags",
    "vfs",
    "nativeCommands",
    "projectTemp",
    "alwaysTranspile",
    "tasks",
    "external",
];

/// Network permission: all hosts, or a host list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum NetPermission {
    All(bool),
    Hosts(Vec<String>),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionsConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deny_read: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deny_write: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub net: Option<NetPermission>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_read_all: Option<bool>,
}

/// Approval channel names this version knows.
pub const KNOWN_APPROVAL_CHANNELS: &[&str] = &["toa", "ask", "tmux", "gui"];

/// Interactive approval settings.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalConfig {
    /// Channels to try in order. An empty list means text only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channels: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_sec: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_timeout_sec: Option<u64>,
}

/// [`ApprovalConfig`] with defaults filled in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedApproval {
    pub channels: Vec<String>,
    pub timeout: Duration,
    pub runtime_timeout: Duration,
}

impl ApprovalConfig {
    pub const DEFAULT_TIMEOUT_SEC: u64 = 60;
    pub const DEFAULT_RUNTIME_TIMEOUT_SEC: u64 = 45;

    /// Concrete values, using defaults for anything unset.
    #[must_use]
    pub fn resolved(cfg: Option<&ApprovalConfig>) -> ResolvedApproval {
        let cfg = cfg.cloned().unwrap_or_default();
        ResolvedApproval {
            channels: cfg.channels.unwrap_or_else(|| {
                KNOWN_APPROVAL_CHANNELS
                    .iter()
                    .map(|c| (*c).to_string())
                    .collect()
            }),
            timeout: Duration::from_secs(cfg.timeout_sec.unwrap_or(Self::DEFAULT_TIMEOUT_SEC)),
            runtime_timeout: Duration::from_secs(
                cfg.runtime_timeout_sec
                    .unwrap_or(Self::DEFAULT_RUNTIME_TIMEOUT_SEC),
            ),
        }
    }
}

/// Contents of `config.json` (global or project).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    /// Alias of `workspace_dir` that loading fills in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_roots: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_project_dir_write: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_home_in_default_read: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_path_dirs_in_default_read: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_project_commands: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passthrough_analyzable: Option<bool>,
    /// Default timeout in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permissions: Option<PermissionsConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<EnvConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval: Option<ApprovalConfig>,
    /// Argv rules: allow, ask or deny by argv prefix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules: Option<Vec<crate::rules::Rule>>,
    /// Keys this version does not know, including dropped ones. Kept so saves do not lose them.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// One entry of `allowedCommands` in `config.local.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AllowedCommand {
    Name(String),
    Named {
        name: String,
        path: String,
    },
    Command {
        command: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subcommands: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        flags: Option<Vec<String>>,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalPermissions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write: Option<Vec<String>>,
}

/// Contents of `config.local.json`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_commands: Option<Vec<AllowedCommand>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permissions: Option<LocalPermissions>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl LocalConfig {
    /// Convert `allowedCommands` into `permissions.run`. Only plain strings count, as in TS.
    /// Object entries came from `config.local.ts` and are ignored with a warning.
    #[must_use]
    pub fn to_config(&self, label: &str) -> (Option<Config>, Vec<String>) {
        let mut run: Vec<String> = Vec::new();
        let mut warnings = Vec::new();
        for entry in self.allowed_commands.iter().flatten() {
            match entry {
                AllowedCommand::Name(name) => run.push(name.clone()),
                AllowedCommand::Named { name, .. } => warnings.push(format!(
                    "{label}: ignored object entry '{name}'; only plain command strings are allowed"
                )),
                AllowedCommand::Command { command, .. } => warnings.push(format!(
                    "{label}: ignored object entry '{command}'; only plain command strings are allowed"
                )),
            }
        }
        // Path grants from `retry-path` "always". TS wrote but never loaded them.
        let local = self.permissions.clone().unwrap_or_default();
        let non_empty = |v: Option<Vec<String>>| v.filter(|v| !v.is_empty());
        let (read, write) = (non_empty(local.read), non_empty(local.write));
        if run.is_empty() && read.is_none() && write.is_none() {
            return (None, warnings);
        }
        let config = Config {
            permissions: Some(PermissionsConfig {
                run: (!run.is_empty()).then_some(run),
                read,
                write,
                ..PermissionsConfig::default()
            }),
            ..Config::default()
        };
        (Some(config), warnings)
    }
}

/// Warnings for keys in `extra`: dropped keys and unknown keys.
#[must_use]
pub fn extra_key_warnings(label: &str, extra: &BTreeMap<String, Value>) -> Vec<String> {
    extra
        .keys()
        .map(|key| {
            if DROPPED_KEYS.contains(&key.as_str()) {
                format!("{label}: '{key}' is no longer supported and is ignored")
            } else {
                format!("{label}: unknown key '{key}' is ignored")
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_approval_channels_contains_toa() {
        assert!(
            KNOWN_APPROVAL_CHANNELS.contains(&"toa"),
            "KNOWN_APPROVAL_CHANNELS should include 'toa'"
        );
        assert_eq!(KNOWN_APPROVAL_CHANNELS, &["toa", "ask", "tmux", "gui"]);
    }

    #[test]
    fn approval_config_resolved_defaults_include_toa() {
        let resolved = ApprovalConfig::resolved(None);
        assert_eq!(resolved.channels, vec!["toa", "ask", "tmux", "gui"]);
    }

    #[test]
    fn approval_config_resolved_custom_channels_with_toa() {
        let cfg = ApprovalConfig {
            channels: Some(vec![
                "toa".to_string(),
                "tmux".to_string(),
                "gui".to_string(),
                "ask".to_string(),
            ]),
            timeout_sec: Some(30),
            runtime_timeout_sec: Some(15),
        };
        let resolved = ApprovalConfig::resolved(Some(&cfg));
        assert_eq!(resolved.channels, vec!["toa", "tmux", "gui", "ask"]);
        assert_eq!(resolved.timeout, Duration::from_secs(30));
        assert_eq!(resolved.runtime_timeout, Duration::from_secs(15));
    }
}
