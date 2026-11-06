use serde::Serialize;

use crate::{Agent, HookInput};

/// What the caller decided for this tool call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Let the agent run the command unchanged.
    Passthrough,
    /// Replace the command. `command` is final, including any env prefix or cwd trailer.
    Rewrite { command: String },
    /// Block the command and show `message` to the model.
    Deny { message: String },
}

/// Text for stdout plus the process exit code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    pub stdout: String,
    pub exit_code: i32,
}

const REWRITE_REASON: &str = "Routed through safesh";

#[derive(Serialize)]
struct Wrapped<'a> {
    #[serde(rename = "hookSpecificOutput")]
    hook_specific_output: Specific<'a>,
}

#[derive(Serialize)]
struct Specific<'a> {
    #[serde(rename = "hookEventName")]
    hook_event_name: &'a str,
    #[serde(rename = "permissionDecision")]
    permission_decision: &'a str,
    #[serde(rename = "permissionDecisionReason")]
    permission_decision_reason: &'a str,
    #[serde(rename = "updatedInput", skip_serializing_if = "Option::is_none")]
    updated_input: Option<UpdatedInput<'a>>,
}

#[derive(Serialize)]
struct UpdatedInput<'a> {
    command: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    timeout: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    run_in_background: Option<bool>,
}

#[derive(Serialize)]
struct Agy<'a> {
    decision: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    overwrite: Option<Overwrite<'a>>,
}

#[derive(Serialize)]
struct Overwrite<'a> {
    #[serde(rename = "CommandLine")]
    command_line: &'a str,
}

fn line<T: Serialize>(v: &T) -> String {
    let mut s = serde_json::to_string(v).expect("hook output serializes");
    s.push('\n');
    s
}

/// Render `outcome` in the output protocol of `agent`.
#[must_use]
pub fn format_output(agent: Agent, input: &HookInput, outcome: &Outcome) -> Rendered {
    let stdout = match (agent, outcome) {
        (Agent::Agy, Outcome::Passthrough) => line(&Agy {
            decision: "allow",
            reason: None,
            overwrite: None,
        }),
        (_, Outcome::Passthrough) => String::new(),
        (Agent::Agy, Outcome::Rewrite { command }) => line(&Agy {
            decision: "allow",
            reason: Some(REWRITE_REASON),
            overwrite: Some(Overwrite {
                command_line: command,
            }),
        }),
        (Agent::Agy, Outcome::Deny { message }) => line(&Agy {
            decision: "deny",
            reason: Some(message),
            overwrite: None,
        }),
        (_, Outcome::Rewrite { command }) => wrapped(
            input,
            "allow",
            REWRITE_REASON,
            Some(UpdatedInput {
                command,
                timeout: input.timeout,
                // The agent only needs the flag when it is set.
                run_in_background: input.run_in_background.then_some(true),
            }),
        ),
        (_, Outcome::Deny { message }) => wrapped(input, "deny", message, None),
    };
    Rendered {
        stdout,
        exit_code: 0,
    }
}

fn wrapped(
    input: &HookInput,
    decision: &str,
    reason: &str,
    updated_input: Option<UpdatedInput<'_>>,
) -> String {
    let event = input
        .hook_event_name
        .as_deref()
        .filter(|e| !e.is_empty())
        .unwrap_or("PreToolUse");
    line(&Wrapped {
        hook_specific_output: Specific {
            hook_event_name: event,
            permission_decision: decision,
            permission_decision_reason: reason,
            updated_input,
        },
    })
}
