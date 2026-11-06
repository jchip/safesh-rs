//! Config merge: arrays union, scalars last-set-wins.

use super::types::{Config, EnvConfig, NetPermission, PermissionsConfig};

/// Union keeping first-seen order.
fn union(a: Option<&Vec<String>>, b: Option<&Vec<String>>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for item in a.into_iter().chain(b).flatten() {
        if !out.contains(item) {
            out.push(item.clone());
        }
    }
    out
}

fn hosts(n: Option<&NetPermission>) -> Option<&Vec<String>> {
    match n {
        Some(NetPermission::Hosts(h)) => Some(h),
        _ => None,
    }
}

fn merge_net(base: Option<&NetPermission>, over: Option<&NetPermission>) -> NetPermission {
    let all = |n: Option<&NetPermission>| matches!(n, Some(NetPermission::All(true)));
    if all(base) || all(over) {
        return NetPermission::All(true);
    }
    NetPermission::Hosts(union(hosts(base), hosts(over)))
}

fn merge_permissions(base: &PermissionsConfig, over: &PermissionsConfig) -> PermissionsConfig {
    PermissionsConfig {
        read: Some(union(base.read.as_ref(), over.read.as_ref())),
        deny_read: Some(union(base.deny_read.as_ref(), over.deny_read.as_ref())),
        write: Some(union(base.write.as_ref(), over.write.as_ref())),
        deny_write: Some(union(base.deny_write.as_ref(), over.deny_write.as_ref())),
        net: Some(merge_net(base.net.as_ref(), over.net.as_ref())),
        run: Some(union(base.run.as_ref(), over.run.as_ref())),
        env: Some(union(base.env.as_ref(), over.env.as_ref())),
    }
}

fn merge_env(base: &EnvConfig, over: &EnvConfig) -> EnvConfig {
    EnvConfig {
        allow: Some(union(base.allow.as_ref(), over.allow.as_ref())),
        mask: Some(union(base.mask.as_ref(), over.mask.as_ref())),
        allow_read_all: over.allow_read_all.or(base.allow_read_all),
    }
}

/// Merge `over` onto `base`. Neither input changes.
#[must_use]
pub fn merge_configs(base: &Config, over: &Config) -> Config {
    let pick = |b: &Option<String>, o: &Option<String>| o.clone().or_else(|| b.clone());
    let mut extra = base.extra.clone();
    extra.extend(over.extra.clone());
    Config {
        workspace: pick(&base.workspace, &over.workspace),
        workspace_dir: pick(&base.workspace_dir, &over.workspace_dir),
        project_dir: pick(&base.project_dir, &over.project_dir),
        workspace_roots: Some(union(
            base.workspace_roots.as_ref(),
            over.workspace_roots.as_ref(),
        )),
        block_project_dir_write: over
            .block_project_dir_write
            .or(base.block_project_dir_write),
        include_home_in_default_read: over
            .include_home_in_default_read
            .or(base.include_home_in_default_read),
        include_path_dirs_in_default_read: over
            .include_path_dirs_in_default_read
            .or(base.include_path_dirs_in_default_read),
        allow_project_commands: over.allow_project_commands.or(base.allow_project_commands),
        passthrough_analyzable: over.passthrough_analyzable.or(base.passthrough_analyzable),
        timeout: over.timeout.or(base.timeout),
        permissions: Some(merge_permissions(
            &base.permissions.clone().unwrap_or_default(),
            &over.permissions.clone().unwrap_or_default(),
        )),
        env: Some(merge_env(
            &base.env.clone().unwrap_or_default(),
            &over.env.clone().unwrap_or_default(),
        )),
        extra,
    }
}
