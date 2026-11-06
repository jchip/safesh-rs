//! Whole-exec decisions: run check, spec-table paths and argv rules together.

use std::path::{Path, PathBuf};

use super::Policy;
use crate::effect::{Access, Decision};
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
                    if is_outer && rule == Some(RuleAction::Allow) {
                        continue;
                    }
                    let target = match program {
                        Some(p) if is_outer => p.to_string_lossy().into_owned(),
                        _ => name,
                    };
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

    /// Decides a pipeline seen whole by the static pre-check, for `|`-shaped rules.
    #[must_use]
    pub fn check_pipeline(&self, stages: &[Vec<String>]) -> Option<RuleAction> {
        rules::evaluate_pipeline(&self.rules, stages)
    }
}
