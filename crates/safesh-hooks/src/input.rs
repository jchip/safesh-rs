use std::path::PathBuf;

use serde_json::Value;

use crate::Agent;

/// Shell tool call read from hook stdin.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HookInput {
    pub tool_name: String,
    pub command: String,
    pub cwd: Option<PathBuf>,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    /// Timeout as the agent gave it (agy `WaitMsBeforeAsync` is converted to seconds).
    pub timeout: Option<u64>,
    pub run_in_background: bool,
    pub hook_event_name: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum HookError {
    #[error("hook input is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
}

fn is_shell_tool(name: &str) -> bool {
    ["bash", "run_shell_command", "run_command"]
        .iter()
        .any(|t| name.eq_ignore_ascii_case(t))
}

/// First of `keys` in `obj` that is not null, as in JS `??`.
fn first<'a>(sources: &[(Option<&'a Value>, &[&str])]) -> Option<&'a Value> {
    sources.iter().find_map(|(obj, keys)| {
        let obj = (*obj)?;
        keys.iter()
            .find_map(|k| obj.get(*k).filter(|v| !v.is_null()))
    })
}

/// First non-empty string among `keys`, as in JS `||`.
fn first_str(obj: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|k| obj.get(*k)?.as_str())
        .find(|s| !s.is_empty())
        .map(str::to_string)
}

/// Parse hook stdin. `None` means the call is not a shell tool call to act on.
///
/// `agent` is accepted for symmetry with [`crate::format_output`]. All agents share one input shape.
pub fn parse_input(_agent: Agent, stdin: &str) -> Result<Option<HookInput>, HookError> {
    let root: Value = serde_json::from_str(stdin)?;
    let tool_input = root.get("tool_input").or_else(|| root.get("toolInput"));
    let tool_call = root.get("toolCall");
    let args = tool_call.and_then(|c| c.get("args"));

    let mut tool_name = first_str(&root, &["tool_name", "toolName"]).unwrap_or_default();
    if tool_name.is_empty() {
        tool_name = tool_call
            .and_then(|c| first_str(c, &["name"]))
            .unwrap_or_default();
    }
    if !is_shell_tool(&tool_name) {
        return Ok(None);
    }

    let command = first(&[
        (tool_input, &["command", "CommandLine"]),
        (args, &["CommandLine", "command"]),
    ])
    .and_then(Value::as_str)
    .filter(|c| !c.is_empty());
    let Some(command) = command else {
        return Ok(None);
    };

    let num = |v: &Value| v.as_f64().filter(|n| n.is_finite() && *n >= 0.0);
    // Truncation is fine: timeouts are small whole seconds.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let timeout = first(&[(tool_input, &["timeout"]), (args, &["timeout"])])
        .and_then(num)
        .or_else(|| {
            args.and_then(|a| a.get("WaitMsBeforeAsync"))
                .and_then(num)
                .filter(|ms| *ms != 0.0)
                .map(|ms| (ms / 1000.0).round())
        })
        .map(|n| n as u64);

    let run_in_background = first(&[
        (tool_input, &["run_in_background", "runInBackground"]),
        (args, &["run_in_background", "runInBackground"]),
    ])
    .and_then(Value::as_bool)
    .unwrap_or(false);

    let cwd = first(&[
        (args, &["Cwd", "cwd"]),
        (tool_input, &["cwd", "Cwd"]),
        (Some(&root), &["cwd"]),
    ])
    .and_then(Value::as_str)
    .filter(|s| !s.is_empty())
    .map(PathBuf::from);

    let session_id = first_str(
        &root,
        &[
            "session_id",
            "sessionId",
            "conversationId",
            "conversation_id",
        ],
    );
    let turn_id = first_str(&root, &["turn_id", "turnId"]).or_else(|| {
        root.get("stepIdx")
            .filter(|v| !v.is_null())
            .map(|v| match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            })
    });

    Ok(Some(HookInput {
        tool_name,
        command: command.to_string(),
        cwd,
        session_id,
        turn_id,
        timeout,
        run_in_background,
        hook_event_name: first_str(&root, &["hook_event_name", "hookEventName"]),
    }))
}
