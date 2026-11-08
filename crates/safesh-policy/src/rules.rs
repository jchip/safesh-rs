//! Argv rules: prefix patterns on the expanded argv with allow, ask or deny.
//!
//! Pattern elements match one argv token each, with `*` and `?` globs inside a token.
//! `**` matches any number of tokens. A `|` element splits a pipeline-shaped rule into stages;
//! those are only matched by the static pre-check, which sees whole pipelines.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleAction {
    Allow,
    Ask,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    #[serde(rename = "match")]
    pub pattern: Vec<String>,
    pub action: RuleAction,
}

/// Strongest action among rules whose pattern has no `|` and matches `argv`.
#[must_use]
pub fn evaluate(rules: &[Rule], argv: &[String]) -> Option<RuleAction> {
    rules
        .iter()
        .filter(|r| !r.pattern.iter().any(|p| p == "|"))
        .filter(|r| match_prefix(&r.pattern, argv, true))
        .map(|r| r.action)
        .max()
}

/// Like [`evaluate`], but a rule's leading element must match `argv[0]` as written. The basename
/// rule is off, so a `git` pattern does not match `/usr/bin/git`.
#[must_use]
pub fn evaluate_literal(rules: &[Rule], argv: &[String]) -> Option<RuleAction> {
    rules
        .iter()
        .filter(|r| !r.pattern.iter().any(|p| p == "|"))
        .filter(|r| match_prefix(&r.pattern, argv, false))
        .map(|r| r.action)
        .max()
}

/// Strongest action among `|` rules whose stage patterns match consecutive pipeline stages.
#[must_use]
pub fn evaluate_pipeline(rules: &[Rule], stages: &[Vec<String>]) -> Option<RuleAction> {
    rules
        .iter()
        .filter(|r| r.pattern.iter().any(|p| p == "|"))
        .filter(|r| {
            let pats = split_stages(&r.pattern);
            !pats.iter().any(|p| p.is_empty()) && pipeline_matches(&pats, stages)
        })
        .map(|r| r.action)
        .max()
}

/// Error messages for malformed rules. Empty when all rules are valid.
#[must_use]
pub fn validate(rules: &[Rule]) -> Vec<String> {
    let mut errs = Vec::new();
    for (i, r) in rules.iter().enumerate() {
        let p = &r.pattern;
        if p.is_empty() {
            errs.push(format!("rules[{i}]: empty match pattern"));
            continue;
        }
        if p[0] == "|" {
            errs.push(format!("rules[{i}]: pattern must not start with `|`"));
            continue;
        }
        if p[0] == "**" {
            errs.push(format!("rules[{i}]: pattern must not start with `**`"));
            continue;
        }
        if split_stages(p).iter().any(|s| s.is_empty()) {
            errs.push(format!("rules[{i}]: empty stage around `|`"));
        }
    }
    errs
}

fn split_stages(pattern: &[String]) -> Vec<&[String]> {
    pattern.split(|p| p == "|").collect()
}

fn pipeline_matches(pats: &[&[String]], stages: &[Vec<String>]) -> bool {
    if pats.len() > stages.len() {
        return false;
    }
    (0..=stages.len() - pats.len()).any(|start| {
        pats.iter()
            .zip(&stages[start..])
            .all(|(p, s)| match_prefix(p, s, true))
    })
}

/// Prefix match. `first` enables the basename rule for the leading element.
fn match_prefix(pat: &[String], argv: &[String], first: bool) -> bool {
    let Some((head, rest)) = pat.split_first() else {
        return true;
    };
    if head == "**" {
        return (0..=argv.len()).any(|k| match_prefix(rest, &argv[k..], false));
    }
    let Some((tok, tail)) = argv.split_first() else {
        return false;
    };
    let ok = glob_match(head, tok)
        || (first && tok.contains('/') && glob_match(head, tok.rsplit('/').next().unwrap_or(tok)));
    ok && match_prefix(rest, tail, false)
}

enum Piece {
    Lit(char),
    Any,
    One,
}

fn compile(glob: &str) -> Vec<Piece> {
    let mut out = Vec::new();
    let mut chars = glob.chars();
    while let Some(c) = chars.next() {
        out.push(match c {
            '*' => Piece::Any,
            '?' => Piece::One,
            '\\' => Piece::Lit(chars.next().unwrap_or('\\')),
            c => Piece::Lit(c),
        });
    }
    out
}

fn glob_match(glob: &str, text: &str) -> bool {
    let text: Vec<char> = text.chars().collect();
    match_pieces(&compile(glob), &text)
}

fn match_pieces(p: &[Piece], t: &[char]) -> bool {
    match p.split_first() {
        None => t.is_empty(),
        Some((Piece::Any, rest)) => (0..=t.len()).any(|k| match_pieces(rest, &t[k..])),
        Some((Piece::One, rest)) => !t.is_empty() && match_pieces(rest, &t[1..]),
        Some((Piece::Lit(c), rest)) => t.first() == Some(c) && match_pieces(rest, &t[1..]),
    }
}
