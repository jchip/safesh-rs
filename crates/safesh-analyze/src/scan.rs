//! Cheap syntactic pre-scan of a subtree. Used to invalidate state before walking loops and
//! to summarize function bodies.

use std::collections::BTreeSet;

use brush_parser::ast::{
    self, Command, CommandPrefixOrSuffixItem as Item, CompoundCommand, CompoundList,
    CompoundListItem,
};

use crate::builtins::{is_identifier, is_var_writer};

#[derive(Default)]
pub(crate) struct Scan {
    /// Variable names that may be written.
    pub assigned: BTreeSet<String>,
    /// Literal command names.
    pub names: BTreeSet<String>,
    /// A command name that is not a plain literal.
    pub dynamic: bool,
}

/// Command word text when it has no expansions. Plain quotes are removed.
pub(crate) fn literal(raw: &str) -> Option<String> {
    if raw.contains(['$', '`', '\\', '*', '?', '[', '{', '(', '~']) {
        return None;
    }
    if !raw.contains(['\'', '"']) {
        return Some(raw.to_string());
    }
    let b = raw.as_bytes();
    if raw.len() >= 2 && (b[0] == b'\'' || b[0] == b'"') && b[0] == b[raw.len() - 1] {
        let inner = &raw[1..raw.len() - 1];
        if !inner.contains(['\'', '"']) {
            return Some(inner.to_string());
        }
    }
    None
}

pub(crate) fn assignment_name(a: &ast::Assignment) -> &str {
    match &a.name {
        ast::AssignmentName::VariableName(n) | ast::AssignmentName::ArrayElementName(n, _) => n,
    }
}

/// True when arithmetic text may assign a variable.
fn arith_assigns(text: &str) -> bool {
    if text.contains("++") || text.contains("--") {
        return true;
    }
    let b: Vec<char> = text.chars().collect();
    for (i, c) in b.iter().enumerate() {
        if *c == '=' {
            let prev = i.checked_sub(1).map(|j| b[j]);
            let next = b.get(i + 1).copied();
            if next != Some('=') && !matches!(prev, Some('=' | '!' | '<' | '>')) {
                return true;
            }
        }
    }
    false
}

/// Identifiers in arithmetic text when it may assign.
pub(crate) fn arith_targets(text: &str) -> Vec<String> {
    if !arith_assigns(text) {
        return Vec::new();
    }
    text.split(|c: char| !(c == '_' || c.is_ascii_alphanumeric()))
        .filter(|t| is_identifier(t))
        .map(str::to_string)
        .collect()
}

impl Scan {
    fn arith(&mut self, text: &str) {
        self.assigned.extend(arith_targets(text));
    }
}

pub(crate) fn scan_list(l: &CompoundList, s: &mut Scan) {
    for CompoundListItem(ao, _) in &l.0 {
        for (_, p) in ao {
            for c in &p.seq {
                scan_command(c, s);
            }
        }
    }
}

fn scan_command(c: &Command, s: &mut Scan) {
    match c {
        Command::Simple(sc) => scan_simple(sc, s),
        Command::Compound(cc, _) => scan_compound(cc, s),
        Command::Function(_) => {}
    }
}

pub(crate) fn scan_compound(cc: &CompoundCommand, s: &mut Scan) {
    match cc {
        CompoundCommand::Arithmetic(a) => s.arith(&a.expr.value),
        CompoundCommand::ArithmeticForClause(f) => {
            for e in [&f.initializer, &f.condition, &f.updater]
                .into_iter()
                .flatten()
            {
                s.arith(&e.value);
            }
            scan_list(&f.body.list, s);
        }
        CompoundCommand::BraceGroup(b) => scan_list(&b.list, s),
        CompoundCommand::ForClause(f) => {
            s.assigned.insert(f.variable_name.clone());
            scan_list(&f.body.list, s);
        }
        CompoundCommand::CaseClause(c) => {
            for item in &c.cases {
                if let Some(l) = &item.cmd {
                    scan_list(l, s);
                }
            }
        }
        CompoundCommand::IfClause(i) => {
            scan_list(&i.condition, s);
            scan_list(&i.then, s);
            for e in i.elses.iter().flatten() {
                if let Some(c) = &e.condition {
                    scan_list(c, s);
                }
                scan_list(&e.body, s);
            }
        }
        CompoundCommand::WhileClause(w) | CompoundCommand::UntilClause(w) => {
            scan_list(&w.0, s);
            scan_list(&w.1.list, s);
        }
        CompoundCommand::Coprocess(c) => scan_command(&c.body, s),
        // A subshell cannot change the parent's variables or cwd; a test changes nothing.
        CompoundCommand::Subshell(_) | CompoundCommand::ExtendedTest(_) => {}
    }
}

fn scan_simple(sc: &ast::SimpleCommand, s: &mut Scan) {
    let items = sc
        .prefix
        .iter()
        .flat_map(|p| p.0.iter())
        .chain(sc.suffix.iter().flat_map(|p| p.0.iter()));
    for it in items {
        if let Item::AssignmentWord(a, _) = it {
            s.assigned.insert(assignment_name(a).to_string());
        }
    }
    let Some(name) = &sc.word_or_name else { return };
    let lit = if name.value == "[" {
        Some("[".to_string())
    } else {
        literal(&name.value)
    };
    match lit {
        None => s.dynamic = true,
        Some(n) => {
            if is_var_writer(&n) {
                for it in sc.suffix.iter().flat_map(|p| p.0.iter()) {
                    if let Item::Word(w) = it
                        && let Some(l) = literal(&w.value)
                    {
                        if is_identifier(&l) {
                            s.assigned.insert(l.clone());
                        }
                        s.arith(&l);
                    }
                }
            }
            s.names.insert(n);
        }
    }
}
