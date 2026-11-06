use std::str::FromStr;

/// Agent CLI that invokes the hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agent {
    Claude,
    Gemini,
    Codex,
    Agy,
}

#[derive(Debug, thiserror::Error)]
#[error("unknown agent `{0}`, expected claude, gemini, codex or agy")]
pub struct ParseAgentError(String);

impl FromStr for Agent {
    type Err = ParseAgentError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "claude" => Ok(Self::Claude),
            "gemini" => Ok(Self::Gemini),
            "codex" => Ok(Self::Codex),
            "agy" => Ok(Self::Agy),
            other => Err(ParseAgentError(other.to_string())),
        }
    }
}
