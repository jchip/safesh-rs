//! Agent hook protocols: parse `PreToolUse` input, format allow, rewrite and deny output.
//!
//! This crate has no decision logic. The caller picks an [`Outcome`].

mod agent;
mod approval;
mod input;
mod message;
mod output;

pub use agent::{Agent, ParseAgentError};
pub use approval::{
    ApprovalError, ApprovalRecord, ApprovalRequest, consume_approval,
    format_permission_request_allow, write_approval,
};
pub use input::{HookError, HookInput, parse_input};
pub use message::{blocked_message, path_blocked_message};
pub use output::{Outcome, Rendered, format_output};
