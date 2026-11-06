//! Hermetic harness for running the `safesh` binary.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};
use std::io::Write as _;

pub struct Env {
    _td: tempfile::TempDir,
    pub root: PathBuf,
    pub home: PathBuf,
    pub state: PathBuf,
    pub proj: PathBuf,
}

pub struct Hook {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
    pub json: Option<Value>,
}

impl Hook {
    fn field(&self, k: &str) -> String {
        self.json
            .as_ref()
            .and_then(|j| j["hookSpecificOutput"][k].as_str())
            .unwrap_or_default()
            .to_string()
    }
    pub fn decision(&self) -> String {
        self.field("permissionDecision")
    }
    pub fn reason(&self) -> String {
        self.field("permissionDecisionReason")
    }
    /// The rewritten command, empty when the hook did not rewrite.
    pub fn command(&self) -> String {
        self.json
            .as_ref()
            .and_then(|j| j["hookSpecificOutput"]["updatedInput"]["command"].as_str())
            .unwrap_or_default()
            .to_string()
    }
    pub fn pending_id(&self) -> String {
        pending_id(&self.reason())
    }
}

/// Extracts the id from `--id=<id>` in a blocked message.
pub fn pending_id(text: &str) -> String {
    let i = text.find("--id=").expect("no --id= in text") + 5;
    text[i..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect()
}

pub struct Run {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Run {
    fn of(o: &Output) -> Self {
        Self {
            code: o.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        }
    }
}

impl Env {
    pub fn new() -> Self {
        let td = tempfile::tempdir().expect("tempdir");
        let root = td.path().canonicalize().expect("canonicalize");
        let home = root.join("home");
        let state = root.join("state");
        let proj = root.join("proj");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(root.join("tmp")).unwrap();
        std::fs::create_dir_all(proj.join(".git")).unwrap();
        Self {
            _td: td,
            root,
            home,
            state,
            proj,
        }
    }

    fn apply(&self, c: &mut Command) {
        c.env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", &self.home)
            .env("TMPDIR", self.root.join("tmp"))
            .env("SAFESH_STATE_DIR", &self.state)
            .current_dir(&self.proj);
    }

    pub fn safesh(&self, args: &[&str]) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_safesh"));
        self.apply(&mut c);
        c.args(args);
        c
    }

    pub fn run_safesh(&self, args: &[&str]) -> Run {
        Run::of(&self.safesh(args).output().expect("spawn safesh"))
    }

    /// Runs a shell string, such as a hook rewrite, with `/bin/sh -c` in the project.
    pub fn sh(&self, script: &str) -> Run {
        let mut c = Command::new("/bin/sh");
        self.apply(&mut c);
        c.arg("-c").arg(script);
        Run::of(&c.output().expect("spawn sh"))
    }

    pub fn hook_raw(&self, args: &[&str], stdin: &str) -> Hook {
        let mut child = self
            .safesh(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn hook");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        let o = child.wait_with_output().unwrap();
        let stdout = String::from_utf8_lossy(&o.stdout).into_owned();
        Hook {
            code: o.status.code().unwrap_or(-1),
            json: serde_json::from_str(&stdout).ok(),
            stdout,
            stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        }
    }

    pub fn input(&self, command: &str, session: &str) -> String {
        json!({
            "tool_name": "Bash",
            "tool_input": {"command": command},
            "cwd": self.proj,
            "session_id": session,
        })
        .to_string()
    }

    pub fn hook_as(&self, agent: &str, command: &str, session: &str) -> Hook {
        self.hook_raw(&["hook", agent], &self.input(command, session))
    }

    pub fn hook(&self, command: &str) -> Hook {
        self.hook_as("claude", command, "s1")
    }

    /// Hook, then run the rewrite. Panics if the hook did not rewrite.
    pub fn hook_run(&self, command: &str) -> Run {
        let h = self.hook(command);
        assert_eq!(
            h.decision(),
            "allow",
            "not rewritten: {} {}",
            h.stdout,
            h.stderr
        );
        self.sh(&h.command())
    }

    pub fn write_global_config(&self, text: &str) {
        let p = self.home.join(".config/safesh/config.json");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    pub fn write(&self, rel: &str, text: &str) {
        let p = self.proj.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    pub fn read_proj(&self, rel: &str) -> String {
        std::fs::read_to_string(self.proj.join(rel)).unwrap_or_default()
    }
}

/// Fails if the blocked target exists.
pub fn assert_absent(p: impl AsRef<Path>) {
    let p = p.as_ref();
    assert!(
        p.symlink_metadata().is_err(),
        "leak: {} exists",
        p.display()
    );
}
