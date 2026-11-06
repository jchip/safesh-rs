//! Config validation: hard errors and advisory warnings.

use super::types::{Config, NetPermission};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConfigValidation {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

fn has(list: Option<&Vec<String>>, item: &str) -> bool {
    list.is_some_and(|l| l.iter().any(|x| x == item))
}

#[must_use]
pub fn validate_config(config: &Config) -> ConfigValidation {
    let mut out = ConfigValidation::default();
    let perms = config.permissions.clone().unwrap_or_default();
    let read = perms.read.as_ref();
    let write = perms.write.as_ref();

    if has(read, "/") {
        out.warnings.push(
            "permissions.read: ['/'] allows reading entire filesystem - consider limiting to specific directories".into(),
        );
    }
    for dir in ["/etc", "/var", "/usr", "/System"] {
        if has(read, dir) {
            out.warnings.push(format!(
                "permissions.read: includes '{dir}' which may contain sensitive files"
            ));
        }
    }
    if has(write, "/") {
        out.errors.push(
            "permissions.write: ['/'] is extremely dangerous - never allow root write access"
                .into(),
        );
    }
    for dir in ["/etc", "/var", "/usr", "/bin", "/sbin", "/System"] {
        if has(write, dir) {
            out.errors.push(format!(
                "permissions.write: includes '{dir}' - this can compromise system security"
            ));
        }
    }
    if has(perms.run.as_ref(), "*") {
        out.errors.push(
            "permissions.run: ['*'] is not allowed - must explicitly list allowed commands".into(),
        );
    }
    if perms.net == Some(NetPermission::All(true)) {
        out.warnings.push(
            "permissions.net: true allows unrestricted network access - consider specifying allowed hosts".into(),
        );
    }

    let no_roots = config.workspace_roots.as_ref().is_none_or(Vec::is_empty);
    if config.project_dir.as_deref().is_none_or(str::is_empty)
        && config.workspace_dir.as_deref().is_none_or(str::is_empty)
        && no_roots
    {
        out.warnings.push(
            "projectDir/workspaceRoots: not set - file permissions will be limited to /tmp and explicit paths".into(),
        );
    }
    out
}
