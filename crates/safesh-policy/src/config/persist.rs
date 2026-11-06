//! Writing config files: `config.local.json` and the global `config.json`.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{Map, Value};

use super::load::{
    ConfigError, get_global_config_json_path, get_local_json_config_path, load_json_config,
    parse_json, read_optional,
};
use super::types::{AllowedCommand, Config, LocalConfig};
use crate::host::HostEnv;

/// Pretty JSON (2 spaces, trailing newline), written through a temp file and rename.
fn write_json_file<T: Serialize>(path: &Path, data: &T) -> Result<(), ConfigError> {
    let write_err = |source| ConfigError::Write {
        path: path.to_path_buf(),
        source,
    };
    let mut content = serde_json::to_string_pretty(data)?;
    content.push('\n');
    if let Some(dir) = path.parent() {
        fs_err::create_dir_all(dir).map_err(write_err)?;
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".tmp.{nanos}"));
    let tmp = std::path::PathBuf::from(tmp);
    let result = fs_err::write(&tmp, content).and_then(|()| fs_err::rename(&tmp, path));
    if let Err(source) = result {
        let _ = fs_err::remove_file(&tmp);
        return Err(write_err(source));
    }
    Ok(())
}

fn sorted_union(existing: impl IntoIterator<Item = String>, add: &[String]) -> Vec<String> {
    let mut all: Vec<String> = existing.into_iter().collect();
    for item in add {
        if !all.contains(item) {
            all.push(item.clone());
        }
    }
    all.sort();
    all
}

/// Add commands to `config.local.json`. Merged list is sorted. Only string entries survive.
pub fn save_to_local_json(project_dir: &Path, commands: &[String]) -> Result<(), ConfigError> {
    let path = get_local_json_config_path(project_dir);
    // A missing or broken file counts as empty.
    let existing = read_optional(&path)
        .ok()
        .flatten()
        .and_then(|t| parse_json::<LocalConfig>(&path, &t).ok());
    let current = existing
        .and_then(|c| c.allowed_commands)
        .into_iter()
        .flatten()
        .filter_map(|c| match c {
            AllowedCommand::Name(s) => Some(s),
            _ => None,
        });
    let local = LocalConfig {
        allowed_commands: Some(
            sorted_union(current, commands)
                .into_iter()
                .map(AllowedCommand::Name)
                .collect(),
        ),
        ..LocalConfig::default()
    };
    write_json_file(&path, &local)
}

/// Add commands to the global `config.json` under `permissions.run`. Merged list is sorted.
/// Other keys, including unknown ones, are kept.
pub fn save_to_global_json(env: &HostEnv, commands: &[String]) -> Result<(), ConfigError> {
    let path = get_global_config_json_path(env);
    let mut config = load_json_config(&path)?.unwrap_or_default();
    let mut permissions = config.permissions.take().unwrap_or_default();
    permissions.run = Some(sorted_union(
        permissions.run.take().unwrap_or_default(),
        commands,
    ));
    let config = Config {
        permissions: Some(permissions),
        ..config
    };
    write_json_file(&path, &config)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConfigUpdate {
    pub commands: Option<Vec<String>>,
    pub read_paths: Option<Vec<String>>,
    pub write_paths: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpdateOptions {
    /// Union with existing values (default) instead of replacing them.
    pub merge: bool,
}

impl Default for UpdateOptions {
    fn default() -> Self {
        Self { merge: true }
    }
}

/// Apply `items` to `obj[key]`: union in insertion order, or replace. Skips empty input.
fn apply_list(obj: &mut Map<String, Value>, key: &str, items: &[String], merge: bool) {
    if items.is_empty() {
        return;
    }
    let mut list: Vec<Value> = match obj.get(key) {
        Some(Value::Array(a)) if merge => a.clone(),
        _ => Vec::new(),
    };
    for item in items {
        let v = Value::String(item.clone());
        if !list.contains(&v) {
            list.push(v);
        }
    }
    obj.insert(key.to_string(), Value::Array(list));
}

/// Update `config.local.json`, keeping unrelated keys. Merged lists keep insertion order.
/// Returns one message per changed list so the caller can report them.
pub fn update_config_local(
    project_dir: &Path,
    update: &ConfigUpdate,
    options: UpdateOptions,
) -> Result<Vec<String>, ConfigError> {
    let path = get_local_json_config_path(project_dir);
    // A missing, broken or non-object file starts fresh.
    let mut root = match read_optional(&path)
        .ok()
        .flatten()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
    {
        Some(Value::Object(m)) => m,
        _ => Map::new(),
    };
    let merge = options.merge;
    let mut messages = Vec::new();

    if let Some(cmds) = update.commands.as_ref().filter(|c| !c.is_empty()) {
        apply_list(&mut root, "allowedCommands", cmds, merge);
        messages.push(format!("commands: {}", cmds.join(", ")));
    }
    if update.read_paths.is_some() || update.write_paths.is_some() {
        let mut perms = match root.remove("permissions") {
            Some(Value::Object(m)) => m,
            _ => Map::new(),
        };
        if let Some(paths) = update.read_paths.as_ref().filter(|p| !p.is_empty()) {
            apply_list(&mut perms, "read", paths, merge);
            messages.push(format!("read: {}", paths.join(", ")));
        }
        if let Some(paths) = update.write_paths.as_ref().filter(|p| !p.is_empty()) {
            apply_list(&mut perms, "write", paths, merge);
            messages.push(format!("write: {}", paths.join(", ")));
        }
        root.insert("permissions".into(), Value::Object(perms));
    }
    write_json_file(&path, &Value::Object(root))?;
    Ok(messages)
}

pub fn add_commands_to_config(commands: &[String], project_dir: &Path) -> Result<(), ConfigError> {
    let update = ConfigUpdate {
        commands: Some(commands.to_vec()),
        ..ConfigUpdate::default()
    };
    update_config_local(project_dir, &update, UpdateOptions::default()).map(|_| ())
}

pub fn add_paths_to_config(
    read_paths: &[String],
    write_paths: &[String],
    project_dir: &Path,
) -> Result<(), ConfigError> {
    let update = ConfigUpdate {
        read_paths: Some(read_paths.to_vec()),
        write_paths: Some(write_paths.to_vec()),
        ..ConfigUpdate::default()
    };
    update_config_local(project_dir, &update, UpdateOptions::default()).map(|_| ())
}
