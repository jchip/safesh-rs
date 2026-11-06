//! Static pre-check: analyze a script and run every known exec and open through the policy.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use safesh_analyze::{Analysis, AnalyzeOptions, Cwd, OpenKind, Word, analyze};
use safesh_policy::effect::{Access, Decision};
use safesh_policy::policy::Problem;

use crate::context::Context;

/// What the pre-check found.
pub struct Evaluation {
    pub analysis: Analysis,
    /// Problems in source order, each with the exec or open it came from.
    pub problems: Vec<(String, Problem)>,
}

impl Evaluation {
    pub fn is_allowed(&self) -> bool {
        self.problems.is_empty()
    }

    /// Cwd to restore after the run, when it is known and differs from `base`.
    pub fn final_cwd(&self, base: &Path) -> Option<PathBuf> {
        if !self.analysis.may_change_cwd {
            return None;
        }
        self.analysis
            .final_cwd
            .clone()
            .filter(|c| c.as_path() != base)
    }
}

fn known(w: &Word) -> Option<&str> {
    match w {
        Word::Known(s) => Some(s),
        Word::Unknown(_) => None,
    }
}

pub fn evaluate(ctx: &Context, script: &str) -> Result<Evaluation, String> {
    let mut env = BTreeMap::new();
    env.insert(
        "HOME".to_string(),
        ctx.host.home.to_string_lossy().into_owned(),
    );
    let opts = AnalyzeOptions {
        base_cwd: ctx.base_cwd.clone(),
        env,
    };
    let analysis = analyze(script, &opts).map_err(|e| e.to_string())?;
    let mut problems = Vec::new();

    for exec in &analysis.execs {
        if exec.builtin || exec.function {
            continue;
        }
        // Unknown name or cwd: the runtime gate decides.
        let (Some(name), Cwd::Known(cwd)) = (exec.argv.first().and_then(known), &exec.cwd) else {
            continue;
        };
        let argv: Option<Vec<String>> = exec
            .argv
            .iter()
            .map(|w| known(w).map(str::to_string))
            .collect();
        let found = match argv {
            Some(argv) => ctx.policy.check_exec(&argv, None, cwd),
            // Some arguments are unknown: check the program now, its paths at runtime.
            None => match ctx.policy.check_run(name, cwd) {
                d if d.is_allow() => Vec::new(),
                decision => vec![Problem::Run {
                    command: name.to_string(),
                    decision,
                }],
            },
        };
        problems.extend(found.into_iter().map(|p| (name.to_string(), p)));
    }

    for open in &analysis.opens {
        let (Some(path), Cwd::Known(cwd)) = (known(&open.path), &open.cwd) else {
            continue;
        };
        let access = match open.kind {
            OpenKind::Source => Access::Read,
            OpenKind::Redirect => open.access,
        };
        let decision = ctx.policy.check_path(Path::new(path), cwd, access);
        if !decision.is_allow() {
            let resolved = match &decision {
                Decision::Allow { resolved }
                | Decision::Ask { resolved, .. }
                | Decision::Deny { resolved, .. } => resolved.clone(),
            };
            problems.push((
                path.to_string(),
                Problem::Path {
                    path: resolved,
                    access,
                    decision,
                },
            ));
        }
    }
    Ok(Evaluation { analysis, problems })
}
