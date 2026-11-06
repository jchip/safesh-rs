//! Config types, defaults, file locations, load and merge, validation and persistence.

mod defaults;
mod load;
mod merge;
mod persist;
mod types;
mod validate;

pub use defaults::{
    DEFAULT_TIMEOUT_MS, SAFE_COMMANDS, SENSITIVE_READ_PATHS, SENSITIVE_WRITE_PATHS, default_config,
};
pub use load::{
    ConfigError, LoadedConfig, get_global_config_dir, get_global_config_json_path,
    get_local_json_config_path, get_project_config_dir, get_project_config_json_path, load_config,
};
pub use merge::merge_configs;
pub use persist::{
    ConfigUpdate, UpdateOptions, add_commands_to_config, add_paths_to_config, save_to_global_json,
    save_to_local_json, update_config_local,
};
pub use types::{
    AllowedCommand, ApprovalConfig, Config, DROPPED_KEYS, EnvConfig, KNOWN_APPROVAL_CHANNELS,
    LocalConfig, LocalPermissions, NetPermission, PermissionsConfig, ResolvedApproval,
};
pub use validate::{ConfigValidation, validate_config};
