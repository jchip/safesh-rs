//! Control-plane detection: calls the hook lets through untouched.
//!
//! Passthrough skips every check, so the whole script must be control-plane calls, optionally
//! joined by `&&`/`||`/`;`, grouped, or followed by a literal `cd` (the hook's own `&& cd '<dir>'`
//! trailer). Every word must be a literal and redirects may only duplicate fds (`2>&1`).
//! Anything else, such as `safesh retry ... | tail`, goes through the normal pre-check,
//! where `safesh` is an allowed command.

use brush_parser::ParserOptions;
use brush_parser::ast::{
    Command, CommandPrefixOrSuffixItem as Item, CompoundCommand, CompoundList, CompoundListItem,
    IoFileRedirectTarget, IoRedirect, SimpleCommand,
};

use crate::scan::literal;
use crate::walk::parse_script;

pub(crate) fn is_control_plane(script: &str) -> bool {
    let Ok(prog) = parse_script(script, &ParserOptions::default()) else {
        return false;
    };
    let mut found = false;
    !prog.complete_commands.is_empty()
        && prog.complete_commands.iter().all(|l| list(l, &mut found))
        && found
}

fn list(l: &CompoundList, found: &mut bool) -> bool {
    l.0.iter().all(|CompoundListItem(ao, _)| {
        ao.iter()
            .all(|(_, p)| p.seq.len() == 1 && p.seq.iter().all(|c| command(c, found)))
    })
}

fn command(c: &Command, found: &mut bool) -> bool {
    match c {
        Command::Simple(sc) => simple(sc, found),
        Command::Compound(CompoundCommand::Subshell(b), None) => list(&b.list, found),
        Command::Compound(CompoundCommand::BraceGroup(b), None) => list(&b.list, found),
        _ => false,
    }
}

/// Literal words of a simple command, or None if any item is not allowed in passthrough.
fn literal_words(sc: &SimpleCommand) -> Option<Vec<String>> {
    for item in sc.prefix.iter().flat_map(|p| p.0.iter()) {
        match item {
            Item::AssignmentWord(_, w) if literal(&w.value).is_some() => {}
            Item::IoRedirect(r) if fd_dup(r) => {}
            _ => return None,
        }
    }
    let mut words = vec![literal(&sc.word_or_name.as_ref()?.value)?];
    for item in sc.suffix.iter().flat_map(|s| s.0.iter()) {
        match item {
            Item::Word(w) => words.push(literal(&w.value)?),
            Item::IoRedirect(r) if fd_dup(r) => {}
            _ => return None,
        }
    }
    Some(words)
}

/// `2>&1`, `>&2`, `3<&-`: no file is opened.
fn fd_dup(r: &IoRedirect) -> bool {
    match r {
        IoRedirect::File(_, _, IoFileRedirectTarget::Fd(_)) => true,
        IoRedirect::File(_, _, IoFileRedirectTarget::Duplicate(w)) => literal(&w.value)
            .is_some_and(|t| t == "-" || (!t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()))),
        _ => false,
    }
}

fn simple(sc: &SimpleCommand, found: &mut bool) -> bool {
    let Some(words) = literal_words(sc) else {
        return false;
    };
    let base = words[0].rsplit('/').next().unwrap_or(&words[0]);
    let args = &words[1..];
    if base == "cd" {
        return args.len() == 1;
    }
    if base != "safesh" && base != "desh" {
        return false;
    }
    let ok = match args.first().map(String::as_str) {
        Some("retry" | "retry-path") => true,
        // A `--pending` run applies a once-grant, so the agent must not be able to type it unchecked.
        Some("run") if base == "safesh" => {
            let has = |f: &str| {
                args.iter()
                    .skip(1)
                    .any(|a| a == f || a.starts_with(&format!("{f}=")))
            };
            has("--id") && !has("--pending")
        }
        _ => false,
    };
    *found |= ok;
    ok
}
