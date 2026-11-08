//! Whole-exec decisions: run check, spec-table paths and argv rules together.

use std::path::{Path, PathBuf};

use super::Policy;
use crate::effect::{Access, Decision};
use crate::paths::resolve_lexical;
use crate::rules::{self, RuleAction};
use crate::spec::{self, Item};

/// One reason an exec is not allowed outright.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// The program is not allowed, or not found.
    Run { command: String, decision: Decision },
    /// A path argument is outside the allow lists or on a deny list.
    Path {
        path: PathBuf,
        access: Access,
        decision: Decision,
    },
    /// An argv rule said ask or deny.
    Rule { action: RuleAction },
    /// The spec table always asks for this shape.
    Ask(String),
}

impl Problem {
    /// A problem approval can't fix: a deny-list or deny-rule hit, or a missing program.
    #[must_use]
    pub fn is_hard(&self) -> bool {
        match self {
            Self::Run { decision, .. } | Self::Path { decision, .. } => {
                matches!(decision, Decision::Deny { .. })
            }
            Self::Rule { action } => *action == RuleAction::Deny,
            Self::Ask(_) => false,
        }
    }
}

impl Policy {
    /// Decides one exec. `program` is the path the shell resolved, when known; it replaces
    /// `argv[0]` for the outer run check (D2). An empty result means allowed.
    #[must_use]
    pub fn check_exec(&self, argv: &[String], program: Option<&Path>, cwd: &Path) -> Vec<Problem> {
        let mut problems = Vec::new();
        let rule = rules::evaluate(&self.rules, argv);
        let mut outer = true;
        for item in spec::interpret(argv) {
            match item {
                Item::Run(name) => {
                    let is_outer = std::mem::take(&mut outer) && argv.first() == Some(&name);
                    let target = match program {
                        Some(p) if is_outer => p.to_string_lossy().into_owned(),
                        _ => name,
                    };
                    if is_outer
                        && rule == Some(RuleAction::Allow)
                        && self.allow_rule_covers(argv, &target, cwd)
                    {
                        continue;
                    }
                    let decision = self.check_run(&target, cwd);
                    if !decision.is_allow() {
                        problems.push(Problem::Run {
                            command: target,
                            decision,
                        });
                    }
                }
                Item::Path { path, access } => {
                    let decision = self.check_path(Path::new(&path), cwd, access);
                    if !decision.is_allow() {
                        let resolved = match &decision {
                            Decision::Allow { resolved }
                            | Decision::Ask { resolved, .. }
                            | Decision::Deny { resolved, .. } => resolved.clone(),
                        };
                        problems.push(Problem::Path {
                            path: resolved,
                            access,
                            decision,
                        });
                    }
                }
                Item::Ask(reason) => problems.push(Problem::Ask(reason)),
            }
        }
        if let Some(action @ (RuleAction::Ask | RuleAction::Deny)) = rule {
            problems.push(Problem::Rule { action });
        }
        problems
    }

    /// Whether an allow rule may stand in for the run check of `target`. Rules match `argv[0]` by
    /// basename, so on its own a `git` rule would let `/any/dir/git` run. It covers only the
    /// program the rule is about (D2):
    /// - a bare name: the runtime check sees it again as the resolved path;
    /// - the program the session's PATH finds for that name;
    /// - a path an allow rule spells out itself.
    fn allow_rule_covers(&self, argv: &[String], target: &str, cwd: &Path) -> bool {
        if !target.contains('/') {
            return true;
        }
        let abs = resolve_lexical(&cwd.to_string_lossy(), target);
        if self.resolves_by_name(&abs, cwd) {
            return true;
        }
        let mut literal = argv.to_vec();
        if let Some(first) = literal.first_mut() {
            first.clone_from(&abs);
        }
        rules::evaluate_literal(&self.rules, &literal) == Some(RuleAction::Allow)
    }

    /// Decides a pipeline seen whole by the static pre-check, for `|`-shaped rules.
    #[must_use]
    pub fn check_pipeline(&self, stages: &[Vec<String>]) -> Option<RuleAction> {
        rules::evaluate_pipeline(&self.rules, stages)
    }
}
