//! Command-spec table and wrapper rules (the Interpret stage).
//!
//! Turns one argv into typed items: the program to run, the paths it reads or writes, and
//! reasons to always ask. Command knowledge lives only here, as data.

use crate::effect::Access;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// A program to run, as written (`git`, `./tool`, `/usr/bin/env`). Wrappers yield their inner program too.
    Run(String),
    /// A path argument, relative to the exec cwd unless absolute.
    Path { path: String, access: Access },
    /// Must ask whatever the lists say. Holds a short reason for the prompt.
    Ask(String),
}

/// Interprets one argv into items. Temporary: runs `argv[0]` only, until the spec table lands.
#[must_use]
pub fn interpret(argv: &[String]) -> Vec<Item> {
    argv.first()
        .map(|p| Item::Run(p.clone()))
        .into_iter()
        .collect()
}
