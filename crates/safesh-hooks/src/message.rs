//! Prompt text shown to the model when safesh blocks a command or path.

use std::fmt::Write;

/// Menu for a blocked command. Choice numbering is the same for every agent.
#[must_use]
pub fn blocked_message(commands: &[String], pending_id: &str) -> String {
    let list = commands.join(", ");
    let mut notes = String::new();
    for c in commands.iter().filter(|c| c.contains('$')) {
        let _ = write!(
            notes,
            "\nNOTE: {c} is a variable used as the command name, and its value isn't known until the command runs. Assign it a plain path earlier in the same command, or write the path out.\n"
        );
    }
    format!(
        "[SAFESH] BLOCKED: {list}
{notes}
WAIT for user choice (1-5):
1. Allow once
2. Always allow (this project)
3. Allow for session
4. Deny
5. Always allow (all projects)

DO NOT SHOW OR REPEAT OPTIONS. AFTER USER RESPONDS: safesh retry --id={pending_id} --choice=<user's choice>"
    )
}

/// Menu for a blocked path. The text does not vary with `write` or `is_dir`: the menu offers
/// r, w and rw, and the directory choice is the `d` suffix.
#[must_use]
pub fn path_blocked_message(path: &str, _write: bool, _is_dir: bool, pending_id: &str) -> String {
    let mut parts: Vec<&str> = path.split('/').collect();
    parts.pop();
    let joined = parts.join("/");
    let dir = if joined.is_empty() { "/" } else { &joined };
    format!(
        "[SAFESH] PATH BLOCKED: {path}

Choose permission (r=read, w=write, rw=both):
File only (add nothing):
  1. Allow once (r1, w1, rw1)
  2. Allow for session (r2, w2, rw2)
  3. Always allow (r3, w3, rw3)

Entire directory {dir}/ (add 'd'):
  1. Allow once (r1d, w1d, rw1d)
  2. Allow for session (r2d, w2d, rw2d)
  3. Always allow (r3d, w3d, rw3d)

4. Deny

AFTER USER RESPONDS: safesh retry-path --id={pending_id} --choice=<user's choice>"
    )
}
