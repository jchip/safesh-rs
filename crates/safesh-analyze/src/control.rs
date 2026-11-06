//! Control-plane detection: calls the hook must let through untouched.

use brush_parser::ParserOptions;
use brush_parser::ast::{
    Command, CommandPrefixOrSuffixItem as Item, CompoundCommand, CompoundList, CompoundListItem,
};

use crate::scan::literal;
use crate::walk::parse_script;

pub(crate) fn is_control_plane(script: &str) -> bool {
    let Ok(prog) = parse_script(script, &ParserOptions::default()) else {
        return false;
    };
    prog.complete_commands.iter().any(list)
}

fn list(l: &CompoundList) -> bool {
    l.0.iter()
        .any(|CompoundListItem(ao, _)| ao.iter().any(|(_, p)| p.seq.iter().any(command)))
}

fn command(c: &Command) -> bool {
    match c {
        Command::Simple(sc) => simple(sc),
        Command::Compound(CompoundCommand::Subshell(b), _) => list(&b.list),
        Command::Compound(CompoundCommand::BraceGroup(b), _) => list(&b.list),
        _ => false,
    }
}

fn simple(sc: &brush_parser::ast::SimpleCommand) -> bool {
    let Some(name) = sc.word_or_name.as_ref().and_then(|w| literal(&w.value)) else {
        return false;
    };
    let base = name.rsplit('/').next().unwrap_or(&name);
    if base != "safesh" && base != "desh" {
        return false;
    }
    let args: Vec<String> = sc
        .suffix
        .iter()
        .flat_map(|s| s.0.iter())
        .filter_map(|i| match i {
            Item::Word(w) => Some(literal(&w.value).unwrap_or_default()),
            _ => None,
        })
        .collect();
    match args.first().map(String::as_str) {
        Some("retry" | "retry-path") => true,
        Some("run") if base == "safesh" => args
            .iter()
            .skip(1)
            .any(|a| a == "--id" || a.starts_with("--id=")),
        _ => false,
    }
}
