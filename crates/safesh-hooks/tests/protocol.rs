use safesh_hooks::{
    Agent, Outcome, blocked_message, format_output, format_permission_request_allow, parse_input,
    path_blocked_message,
};

fn p(s: &str) -> Option<safesh_hooks::HookInput> {
    parse_input(Agent::Claude, s).unwrap()
}

#[test]
fn agent_from_str() {
    assert_eq!("claude".parse::<Agent>().unwrap(), Agent::Claude);
    assert_eq!("gemini".parse::<Agent>().unwrap(), Agent::Gemini);
    assert_eq!("codex".parse::<Agent>().unwrap(), Agent::Codex);
    assert_eq!("agy".parse::<Agent>().unwrap(), Agent::Agy);
    assert!("Claude".parse::<Agent>().is_err());
}

#[test]
fn camel_variants() {
    let i = p(r#"{"toolName":"bash","toolInput":{"command":"ls","runInBackground":true,"timeout":3},"sessionId":"a","turnId":"b","hookEventName":"X"}"#).unwrap();
    assert!(i.run_in_background);
    assert_eq!(i.timeout, Some(3));
    assert_eq!(i.session_id.as_deref(), Some("a"));
    assert_eq!(i.turn_id.as_deref(), Some("b"));
    assert_eq!(i.hook_event_name.as_deref(), Some("X"));
}

#[test]
fn agy_wait_ms_becomes_seconds() {
    let i = p(
        r#"{"toolCall":{"name":"run_command","args":{"command":"ls","WaitMsBeforeAsync":2500}}}"#,
    )
    .unwrap();
    assert_eq!(i.timeout, Some(3));
}

#[test]
fn tool_name_is_case_insensitive_and_command_required() {
    assert!(p(r#"{"tool_name":"BASH","tool_input":{"command":"ls"}}"#).is_some());
    assert!(p(r#"{"tool_name":"Bash","tool_input":{}}"#).is_none());
    assert!(p(r#"{"tool_name":"Bash","tool_input":{"command":""}}"#).is_none());
    assert!(p(r#"{"tool_name":"Edit","tool_input":{"command":"ls"}}"#).is_none());
}

#[test]
fn invalid_json_is_error() {
    assert!(parse_input(Agent::Claude, "ls -la").is_err());
}

#[test]
fn default_event_name() {
    let i = p(r#"{"tool_name":"Bash","tool_input":{"command":"ls"}}"#).unwrap();
    let r = format_output(
        Agent::Claude,
        &i,
        &Outcome::Deny {
            message: "m".into(),
        },
    );
    assert!(r.stdout.contains(r#""hookEventName":"PreToolUse""#));
    assert_eq!(r.exit_code, 0);
}

#[test]
fn rewrite_omits_unset_fields() {
    let i = p(r#"{"tool_name":"Bash","tool_input":{"command":"ls"}}"#).unwrap();
    let r = format_output(
        Agent::Codex,
        &i,
        &Outcome::Rewrite {
            command: "c".into(),
        },
    );
    assert!(
        r.stdout.contains(r#""updatedInput":{"command":"c"}}"#),
        "{}",
        r.stdout
    );
}

#[test]
fn blocked_text() {
    let m = blocked_message(&["curl".into(), "$x".into()], "7-8");
    assert!(m.starts_with("[SAFESH] BLOCKED: curl, $x\n\nNOTE: $x is a variable"));
    assert!(m.ends_with("safesh retry --id=7-8 --choice=<user's choice>"));
    assert!(!m.contains("HINT"));
    assert!(m.contains("5. Always allow (all projects)"));
}

#[test]
fn path_blocked_text() {
    let m = path_blocked_message("/a/b/c.txt", true, false, "9-1");
    assert!(m.starts_with("[SAFESH] PATH BLOCKED: /a/b/c.txt\n"));
    assert!(m.contains("Entire directory /a/b/ (add 'd'):"));
    assert!(m.ends_with("safesh retry-path --id=9-1 --choice=<user's choice>"));
    assert!(
        path_blocked_message("/x", false, false, "i").contains("Entire directory // "),
        "root parent"
    );
}

#[test]
fn permission_request_allow_shape() {
    let v: serde_json::Value = serde_json::from_str(&format_permission_request_allow()).unwrap();
    assert_eq!(v["hookSpecificOutput"]["decision"]["behavior"], "allow");
    assert_eq!(
        v["hookSpecificOutput"]["hookEventName"],
        "PermissionRequest"
    );
}
