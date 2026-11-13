//! `#!` lines that start a shell.

use std::io::Read as _;
use std::path::Path;

/// Whether `program` names `bash`, `sh`, `dash` or `zsh`. A zsh script is expected to be bash
/// and runs in the bash engine. Windows: `bash.exe` is `bash`.
pub fn is_shell(program: &str) -> bool {
    matches!(
        safesh_policy::paths::command_name(program),
        "bash" | "sh" | "dash" | "zsh"
    )
}

/// The shell and its options from `path`'s `#!` line, when that line starts a shell that
/// `is_shell` accepts, directly or through `env`.
pub fn shell_shebang(path: &Path) -> Option<(String, Vec<String>)> {
    let mut head = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(256)
        .read_to_end(&mut head)
        .ok()?;
    parse(&head)
}

fn parse(head: &[u8]) -> Option<(String, Vec<String>)> {
    let line = head.strip_prefix(b"#!")?.split(|&b| b == b'\n').next()?;
    let mut words = std::str::from_utf8(line).ok()?.split_whitespace();
    let mut interp = words.next()?;
    if safesh_policy::paths::command_name(interp) == "env" {
        interp = words.next()?;
        if interp == "-S" {
            interp = words.next()?;
        }
    }
    if !is_shell(interp) {
        return None;
    }
    // A lone `-` only ends the options, which the script path that follows does anyway.
    let opts = words.filter(|w| *w != "-").map(String::from).collect();
    Some((interp.to_string(), opts))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Option<(String, Vec<String>)> {
        parse(s.as_bytes())
    }

    fn shell(interp: &str, opts: &[&str]) -> (String, Vec<String>) {
        (
            interp.into(),
            opts.iter().map(|o| (*o).to_string()).collect(),
        )
    }

    #[test]
    fn direct_shells() {
        assert_eq!(p("#!/bin/sh\necho"), Some(shell("/bin/sh", &[])));
        assert_eq!(p("#! /bin/bash -e\r\n"), Some(shell("/bin/bash", &["-e"])));
        assert_eq!(p("#!/bin/dash -"), Some(shell("/bin/dash", &[])));
        assert_eq!(p("#!/usr/bin/zsh\n"), Some(shell("/usr/bin/zsh", &[])));
    }

    #[test]
    fn through_env() {
        assert_eq!(p("#!/usr/bin/env bash\n"), Some(shell("bash", &[])));
        assert_eq!(p("#!/usr/bin/env zsh\n"), Some(shell("zsh", &[])));
        assert_eq!(
            p("#!/usr/bin/env -S bash -eu\n"),
            Some(shell("bash", &["-eu"]))
        );
    }

    #[test]
    fn not_a_shell() {
        assert_eq!(p("#!/usr/bin/env python3\n"), None);
        assert_eq!(p("#!/usr/bin/fish\n"), None);
        assert_eq!(p("echo no shebang\n"), None);
        assert_eq!(p(""), None);
        assert_eq!(p("#!\n"), None);
    }
}
