//! Static name lists.

/// Bash 5 builtins (`enable -a`) plus `[[`.
const BUILTINS: &[&str] = &[
    ".",
    ":",
    "[",
    "[[",
    "alias",
    "bg",
    "bind",
    "break",
    "builtin",
    "caller",
    "cd",
    "command",
    "compgen",
    "complete",
    "compopt",
    "continue",
    "declare",
    "dirs",
    "disown",
    "echo",
    "enable",
    "eval",
    "exec",
    "exit",
    "export",
    "false",
    "fc",
    "fg",
    "getopts",
    "hash",
    "help",
    "history",
    "jobs",
    "kill",
    "let",
    "local",
    "logout",
    "mapfile",
    "popd",
    "printf",
    "pushd",
    "pwd",
    "read",
    "readarray",
    "readonly",
    "return",
    "set",
    "shift",
    "shopt",
    "source",
    "suspend",
    "test",
    "times",
    "trap",
    "true",
    "type",
    "typeset",
    "ulimit",
    "umask",
    "unalias",
    "unset",
    "wait",
];

pub(crate) fn is_builtin(name: &str) -> bool {
    BUILTINS.contains(&name)
}

/// Builtins whose `NAME=value` arguments assign shell variables.
pub(crate) fn is_assign_builtin(name: &str) -> bool {
    matches!(
        name,
        "export" | "declare" | "local" | "readonly" | "typeset"
    )
}

/// Builtins that write variables named by their plain arguments.
pub(crate) fn is_var_writer(name: &str) -> bool {
    matches!(
        name,
        "read" | "readarray" | "mapfile" | "getopts" | "unset" | "printf" | "let"
    )
}

pub(crate) fn is_identifier(s: &str) -> bool {
    let mut cs = s.chars();
    matches!(cs.next(), Some(c) if c == '_' || c.is_ascii_alphabetic())
        && cs.all(|c| c == '_' || c.is_ascii_alphanumeric())
}
