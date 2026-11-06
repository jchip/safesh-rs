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
