//! Hermetic harness for running the `safesh` binary.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};
use std::io::Write as _;

pub struct Env {
    _td: tempfile::TempDir,
    _outside: tempfile::TempDir,
    pub root: PathBuf,
    /// A writable dir on no allow list. The temp root itself can sit under `/tmp` (Linux),
    /// which is writable by default, so this lives under cargo's per-crate target dir.
    pub outside: PathBuf,
    pub home: PathBuf,
    pub state: PathBuf,
    pub proj: PathBuf,
    /// Put first on `PATH` for the hook and the run. See [`Env::with_path_prefix`].
    pub path_prefix: Option<PathBuf>,
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
        let td = tempfile::tempdir_in(test_tmp_root()).expect("tempdir");
        let root = canon(td.path());
        let home = root.join("home");
        let state = root.join("state");
        let proj = root.join("proj");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(root.join("tmp")).unwrap();
        std::fs::create_dir_all(proj.join(".git")).unwrap();
        let out_td = tempfile::Builder::new()
            .prefix("outside")
            .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
            .expect("outside tempdir");
        let outside = canon(out_td.path());
        Self {
            _td: td,
            _outside: out_td,
            outside,
            root,
            home,
            state,
            proj,
            path_prefix: None,
        }
    }

    /// Puts `dir` first on `PATH` for every process this env starts.
    #[must_use]
    pub fn with_path_prefix(mut self, dir: PathBuf) -> Self {
        self.path_prefix = Some(dir);
        self
    }

    fn apply(&self, c: &mut Command) {
        let mut base = std::env::var_os("PATH").unwrap_or_default();
        // Windows: Git Bash puts its coreutils on PATH, and the agent's shell is Git Bash.
        if cfg!(windows)
            && let Some(git) = &safesh_policy::winpath::mounts().root
        {
            let usr_bin = PathBuf::from(git).join("usr").join("bin");
            base =
                std::env::join_paths(std::iter::once(usr_bin).chain(std::env::split_paths(&base)))
                    .expect("join PATH");
        }
        let path = match &self.path_prefix {
            Some(dir) => std::env::join_paths(
                std::iter::once(dir.clone()).chain(std::env::split_paths(&base)),
            )
            .expect("join PATH"),
            None => base,
        };
        c.env_clear()
            .env("PATH", path)
            .env("HOME", &self.home)
            .env("TMPDIR", self.root.join("tmp"))
            .env("SAFESH_STATE_DIR", &self.state)
            // Never open a real popup or dialog. Approval tests remove this and use fakes.
            .env("SAFESH_APPROVAL", "off")
            .current_dir(&self.proj);
        // Windows processes need these, and safesh reads `TEMP` and `PATHEXT`.
        if cfg!(windows) {
            for k in ["SystemRoot", "windir", "PATHEXT", "COMSPEC"] {
                if let Some(v) = std::env::var_os(k) {
                    c.env(k, v);
                }
            }
            c.env("TEMP", self.root.join("tmp"))
                .env("TMP", self.root.join("tmp"))
                .env("USERPROFILE", &self.home);
        }
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
        let mut c = Command::new(sh_path());
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

/// Writes an executable script at `dir/name` that prints `RAN <name>`, and returns its path.
pub fn fake_tool(dir: &Path, name: &str) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let p = dir.join(name);
    std::fs::write(&p, format!("#!/bin/sh\necho RAN {name}\n")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    p
}

/// Fails if the blocked target exists.
/// Where `Env` roots go. Windows: `%SystemDrive%\\safesh-test-tmp`. The user temp dir is under the
/// profile, and the project search walks past `proj\\.git` to the profile's `.claude`, so every
/// test would share the real profile as its project and write grants there.
pub fn test_tmp_root() -> PathBuf {
    if cfg!(windows) {
        let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
        let root = PathBuf::from(format!("{drive}\\safesh-test-tmp"));
        std::fs::create_dir_all(&root).unwrap();
        return root;
    }
    std::env::temp_dir()
}

/// `/bin/sh`, or Git Bash on Windows, found the way safesh finds the Git install.
pub fn sh_path() -> PathBuf {
    if cfg!(windows) {
        let root = safesh_policy::winpath::mounts()
            .root
            .clone()
            .expect("Git for Windows not found");
        return PathBuf::from(root).join("bin").join("bash.exe");
    }
    PathBuf::from("/bin/sh")
}

/// Real path without the `\\?\` prefix Windows adds, as safesh spells paths.
pub fn canon(p: &Path) -> PathBuf {
    let real = p.canonicalize().expect("canonicalize");
    let s = real.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest),
        _ => real,
    }
}

/// A file the default policy must never let a script write: its spelling in a script, and its
/// path on disk. `/etc/<name>` on Unix, `C:\Windows\<name>` (`/c/Windows/<name>`) on Windows.
pub fn denied_target(name: &str) -> (String, PathBuf) {
    if cfg!(windows) {
        (
            format!("/c/Windows/{name}"),
            PathBuf::from(format!(r"C:\Windows\{name}")),
        )
    } else {
        let p = format!("/etc/{name}");
        (p.clone(), PathBuf::from(p))
    }
}

/// `p` as an unquoted word in a script. Windows: `/` separators, since bash reads `\` as an escape.
pub fn sh_word(p: &Path) -> String {
    let s = p.display().to_string();
    if cfg!(windows) {
        s.replace('\\', "/")
    } else {
        s
    }
}

pub fn assert_absent(p: impl AsRef<Path>) {
    let p = p.as_ref();
    assert!(
        p.symlink_metadata().is_err(),
        "leak: {} exists",
        p.display()
    );
}
