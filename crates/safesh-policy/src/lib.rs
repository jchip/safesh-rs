//! Policy core for safesh: config loading and merging, command and path permissions, session and pending files.
//!
//! The core reads the host environment only through [`host::HostEnv`], so tests inject HOME, TMPDIR and friends
//! instead of mutating process env.

pub mod compat;
pub mod config;
pub mod effect;
pub mod host;
pub mod paths;
pub mod policy;
pub mod project;
pub mod rules;
pub mod scratchpad;
pub mod spec;
pub mod state;
pub mod winpath;
