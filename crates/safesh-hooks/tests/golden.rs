//! Compare rendered output with recorded TS hook output.
//!
//! Goldens were recorded from the TypeScript hooks (`hooks/bash-prehook.ts` and the codex and agy
//! entry points) with an empty HOME. Substitutions applied to the TS side before comparing:
//! - `desh -q --id <h>` becomes `safesh run --id <h>`
//! - `desh retry` becomes `safesh retry`
//! - the rewrite reason becomes `Routed through safesh`
//! - pending ids become `ID`
//! - the trailing `HINT:` paragraph is dropped
//!
//! Codex treats every command as a rewrite candidate, so its `allowed` golden is a rewrite.
//! The TS codex hook also rewrites stdin it cannot parse as a shell call. That quirk is not ported.

use std::path::PathBuf;

use safesh_hooks::{Agent, HookInput, Outcome, blocked_message, format_output, parse_input};
use serde_json::Value;

fn golden(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name)
}

fn read(name: &str) -> String {
    std::fs::read_to_string(golden(name)).unwrap()
}

fn norm_str(s: &str) -> String {
    if s == "Transpiled to SafeShell TypeScript via desh" {
        return "Routed through safesh".into();
    }
    let mut s = s.replace("desh -q --id ", "safesh run --id ");
    s = s.replace("desh retry ", "safesh retry ");
    if let Some(i) = s.find("\n\nHINT:") {
        s.truncate(i);
    }
    if let (Some(a), Some(b)) = (s.find("--id="), s.find(" --choice")) {
        s.replace_range(a + 5..b, "ID");
    }
    s
}

fn norm(v: &Value) -> Value {
    match v {
        Value::String(s) => Value::String(norm_str(s)),
        Value::Array(a) => Value::Array(a.iter().map(norm).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), norm(v))).collect()),
        other => other.clone(),
    }
}

fn agent(name: &str) -> Agent {
    name.parse().unwrap()
}

fn input(a: &str, case: &str) -> HookInput {
    parse_input(agent(a), &read(&format!("{a}-{case}.stdin.json")))
        .unwrap()
        .expect("shell tool")
}

/// Rewritten command from the golden, with the `safesh run` substitution applied.
fn golden_command(a: &str, case: &str) -> String {
    let v: Value = serde_json::from_str(&read(&format!("{a}-{case}.stdout.json"))).unwrap();
    let c = v
        .pointer("/hookSpecificOutput/updatedInput/command")
        .or_else(|| v.pointer("/overwrite/CommandLine"))
        .and_then(Value::as_str)
        .unwrap();
    norm_str(c)
}

fn check(a: &str, case: &str, outcome: &Outcome) {
    let inp = input(a, case);
    let r = format_output(agent(a), &inp, outcome);
    assert_eq!(
        read(&format!("{a}-{case}.exit")).trim(),
        r.exit_code.to_string()
    );
    let want = read(&format!("{a}-{case}.stdout.json"));
    if want.trim().is_empty() {
        assert_eq!(r.stdout, "", "{a}-{case}");
        return;
    }
    assert!(r.stdout.ends_with('\n'));
    let got: Value = serde_json::from_str(&r.stdout).unwrap();
    let want: Value = serde_json::from_str(&want).unwrap();
    assert_eq!(got, norm(&want), "{a}-{case}");
}

const AGENTS: [&str; 4] = ["claude", "gemini", "codex", "agy"];

#[test]
fn allowed() {
    for a in ["claude", "gemini", "agy"] {
        check(a, "allowed", &Outcome::Passthrough);
    }
    let command = golden_command("codex", "allowed");
    check("codex", "allowed", &Outcome::Rewrite { command });
}

#[test]
fn control_plane_passthrough() {
    for a in AGENTS {
        check(a, "control", &Outcome::Passthrough);
    }
}

#[test]
fn blocked() {
    let message = blocked_message(&["foobar".to_string()], "ID");
    for a in AGENTS {
        check(
            a,
            "blocked",
            &Outcome::Deny {
                message: message.clone(),
            },
        );
    }
}

#[test]
fn rewrite() {
    for a in AGENTS {
        let command = golden_command(a, "rewrite");
        check(a, "rewrite", &Outcome::Rewrite { command });
    }
}

#[test]
fn non_shell_tool_is_none() {
    for a in AGENTS {
        let s = read(&format!("{a}-nonshell.stdin.json"));
        assert!(parse_input(agent(a), &s).unwrap().is_none(), "{a}");
    }
    for a in ["claude", "gemini", "agy"] {
        assert_eq!(read(&format!("{a}-nonshell.stdout.json")).trim(), "");
    }
}

#[test]
fn parsed_fields() {
    let c = input("claude", "rewrite");
    assert_eq!(c.timeout, Some(5000));
    assert!(c.run_in_background);
    assert_eq!(c.session_id.as_deref(), Some("s1"));
    assert_eq!(c.cwd, Some("/work".into()));
    let x = input("codex", "allowed");
    assert_eq!(x.turn_id.as_deref(), Some("t1"));
    let y = input("agy", "allowed");
    assert_eq!(y.session_id.as_deref(), Some("c1"));
    assert_eq!(y.turn_id.as_deref(), Some("3"));
    assert_eq!(y.tool_name, "run_command");
    assert_eq!(y.cwd, Some("/work".into()));
}
