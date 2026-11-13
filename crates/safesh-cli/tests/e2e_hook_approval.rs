//! End to end tests for the hook asking the user through a fake tmux popup.
//! Channel binaries come only from `SAFESH_TEST_BIN_DIRS`, so no real tmux or dialog opens.
//!
//! Unix only: the fake tmux is a `#!/bin/sh` script, and tmux is a Unix channel.
#![cfg(unix)]

mod common;
use common::*;

use std::fs;
use std::io::Write as _;
use std::os::unix::fs::PermissionsExt;
use std::process::Stdio;
use std::time::{Duration, Instant};

const FAKE_TMUX: &str = r#"#!/bin/sh
if [ "$1" = "display-message" ]; then echo 1 3.8; exit 0; fi
for a in "$@"; do
  if [ "$a" = "-C" ]; then echo close >> "$FAKE_LOG"; exit 0; fi
done
for last; do :; done
for a in "$@"; do echo "$a" >> "$FAKE_LOG.args"; done
echo "start" >> "$FAKE_LOG"
[ -n "$FAKE_DELAY" ] && sleep "$FAKE_DELAY"
sh -c "$last" < "$FAKE_INPUT" >> "$FAKE_LOG.screen"
echo "end" >> "$FAKE_LOG"
"#;

const TOOL: &str = "./hello.sh";

struct T {
    e: Env,
    bin: std::path::PathBuf,
    log: std::path::PathBuf,
    input: std::path::PathBuf,
}

impl T {
    /// `input` is what the fake popup user types.
    fn new(input: &str) -> Self {
        let e = Env::new();
        let bin = e.root.join("fakebin");
        fs::create_dir(&bin).unwrap();
        let p = bin.join("tmux");
        fs::write(&p, FAKE_TMUX).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
        // Only tmux is listed, so a GUI channel can never run. Sessions otherwise trust project scripts.
        e.write_global_config(r#"{"approval":{"channels":["tmux"]},"allowProjectCommands":false}"#);
        e.write("hello.sh", "#!/bin/sh\necho ran-tool\n");
        fs::set_permissions(e.proj.join("hello.sh"), fs::Permissions::from_mode(0o755)).unwrap();
        let t = Self {
            log: e.root.join("log"),
            input: e.root.join("input"),
            bin,
            e,
        };
        t.type_input(input);
        t
    }

    fn type_input(&self, s: &str) {
        fs::write(&self.input, s).unwrap();
    }

    fn log(&self) -> String {
        fs::read_to_string(&self.log).unwrap_or_default()
    }

    /// The fake popup's args (one per line) and what it showed.
    fn popup(&self) -> (String, String) {
        let read = |ext: &str| fs::read_to_string(self.log.with_extension(ext)).unwrap_or_default();
        (read("args"), read("screen"))
    }

    fn hook_with(&self, args: &[&str], command: &str, session: &str, env: &[(&str, &str)]) -> Hook {
        wait_hook(self.spawn_hook(args, command, session, env))
    }

    fn spawn_hook(
        &self,
        args: &[&str],
        command: &str,
        session: &str,
        env: &[(&str, &str)],
    ) -> std::process::Child {
        let mut c = self.e.safesh(args);
        c.env_remove("SAFESH_APPROVAL")
            .env("SAFESH_TEST_BIN_DIRS", &self.bin)
            .env("TMUX", "/fake,1,0")
            .env("TMUX_PANE", "%1")
            .env("FAKE_LOG", &self.log)
            .env("FAKE_INPUT", &self.input)
            .envs(env.iter().copied());
        let mut child = c
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn hook");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(self.e.input(command, session).as_bytes())
            .unwrap();
        child
    }

    fn hook(&self, command: &str) -> Hook {
        self.hook_with(&["hook", "claude"], command, "s1", &[])
    }
}

fn wait_hook(child: std::process::Child) -> Hook {
    let o = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&o.stdout).into_owned();
    Hook {
        code: o.status.code().unwrap_or(-1),
        json: serde_json::from_str(&stdout).ok(),
        stdout,
        stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
    }
}

/// Value after `flag` in a rewritten command.
fn arg_of(cmd: &str, flag: &str) -> String {
    let mut it = cmd.split_whitespace();
    it.find(|w| *w == flag)
        .and_then(|_| it.next())
        .unwrap_or_else(|| panic!("no {flag} in {cmd}"))
        .to_string()
}

fn pending_files(t: &T) -> usize {
    let n = |d: &str| fs::read_dir(t.e.state.join(d)).map_or(0, Iterator::count);
    n("pending") + n("pending-path")
}

fn assert_text_menu(h: &Hook) {
    assert_eq!(h.decision(), "deny", "{} {}", h.stdout, h.stderr);
    assert!(h.reason().contains("safesh retry --id="), "{}", h.reason());
}

#[test]
fn once_runs_then_is_used_up() {
    let t = T::new("1\n");
    let h = t.hook(TOOL);
    let cmd = h.command();
    assert!(cmd.contains("--pending "), "{} {}", h.stdout, h.stderr);
    let r = t.e.sh(&cmd);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(r.stdout.trim(), "ran-tool");
    let again = t.e.sh(&cmd);
    assert_ne!(again.code, 0, "once-grant reused: {}", again.stdout);
    assert!(!again.stdout.contains("ran-tool"));
    assert!(t.log().contains("start"));
}

#[test]
fn project_answer_saves_to_project_config() {
    let t = T::new("2\n");
    let h = t.hook(TOOL);
    assert!(
        h.command().contains("--pending "),
        "{} {}",
        h.stdout,
        h.stderr
    );
    let cfg = fs::read_to_string(t.e.proj.join(".config/safesh/config.local.json")).unwrap();
    assert!(cfg.contains("hello.sh"), "{cfg}");
    assert_eq!(pending_files(&t), 0);
}

#[test]
fn session_answer_saves_session_grant() {
    let t = T::new("3\n");
    let h = t.hook(TOOL);
    assert!(
        h.command().contains("--pending "),
        "{} {}",
        h.stdout,
        h.stderr
    );
    let s = fs::read_to_string(t.e.proj.join(".temp/safesh/session-s1.json")).unwrap();
    assert!(s.contains("hello.sh"), "{s}");
}

#[test]
fn global_answer_saves_global_config() {
    let t = T::new("5\n");
    let h = t.hook(TOOL);
    assert!(
        h.command().contains("--pending "),
        "{} {}",
        h.stdout,
        h.stderr
    );
    let g = fs::read_to_string(t.e.home.join(".config/safesh/config.json")).unwrap();
    assert!(g.contains("hello.sh"), "{g}");
    assert!(g.contains("tmux"), "approval config lost: {g}");
}

#[test]
fn deny_answer_is_plain_denial() {
    let t = T::new("4\n");
    let h = t.hook(TOOL);
    assert_eq!(h.decision(), "deny", "{}", h.stdout);
    assert!(h.reason().contains("DENIED by the user"), "{}", h.reason());
    assert!(
        !h.reason().to_lowercase().contains("retry"),
        "{}",
        h.reason()
    );
    assert_eq!(pending_files(&t), 0);
}

#[test]
fn no_answer_gives_text_menu() {
    let t = T::new("");
    let h = t.hook(TOOL);
    assert_text_menu(&h);
    assert!(t.log().contains("start"));
}

#[test]
fn approval_off_gives_text_menu_without_tmux() {
    let t = T::new("1\n");
    let h = t.hook_with(
        &["hook", "claude"],
        TOOL,
        "s1",
        &[("SAFESH_APPROVAL", "off")],
    );
    assert_text_menu(&h);
    assert_eq!(t.log(), "");
}

#[test]
fn hard_denial_never_asks() {
    let t = T::new("1\n");
    t.e.write_global_config(
        r#"{"approval":{"channels":["tmux"]},"allowProjectCommands":false,"rules":[{"match":["git","push","--force","**"],"action":"deny"}]}"#,
    );
    let h = t.hook("git push --force origin main");
    assert_eq!(h.decision(), "deny", "{}", h.stdout);
    assert!(h.reason().contains("DENIED"), "{}", h.reason());
    assert_eq!(t.log(), "");
    assert_eq!(pending_files(&t), 0);
}

// Reads outside the workspace are allowed by default, so the path prompt is a write.
#[test]
fn path_write_once_runs() {
    let t = T::new("1\n");
    let f = t.e.outside.join("out.txt");
    let h = t.hook(&format!("echo outside-data > {}", f.display()));
    let cmd = h.command();
    assert!(cmd.contains("--pending "), "{} {}", h.stdout, h.stderr);
    assert!(t.log().contains("start"));
    let (args, screen) = t.popup();
    assert!(
        args.lines()
            .any(|a| a == " safesh: allow writing this path? "),
        "{args}"
    );
    assert!(
        screen.contains(&format!("path: {}", f.display())),
        "{screen}"
    );
    assert!(screen.contains("command: echo outside-data > "), "{screen}");
    assert!(!f.exists());
    let r = t.e.sh(&cmd);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(fs::read_to_string(&f).unwrap().trim(), "outside-data");
}

#[test]
fn once_grant_with_wrong_script_fails() {
    let t = T::new("1\n");
    let cmd = t.hook(TOOL).command();
    let pending = arg_of(&cmd, "--pending");
    let other = t.hook("echo other").command();
    let other_id = arg_of(&other, "--id");
    assert_ne!(other_id, arg_of(&cmd, "--id"));
    let exe = env!("CARGO_BIN_EXE_safesh");
    let r =
        t.e.run_safesh(&["run", "--id", &other_id, "--pending", &pending]);
    assert_ne!(r.code, 0, "{exe}: {}", r.stdout);
    // The failed claim burns the grant, so the right script can't use it either.
    let r = t.e.sh(&cmd);
    assert_ne!(r.code, 0, "{}", r.stdout);
}

#[test]
fn agent_typed_run_pending_is_checked() {
    let t = T::new("");
    let h = t.hook_with(
        &["hook", "claude"],
        "safesh run --id abc --pending def",
        "s1",
        &[("SAFESH_APPROVAL", "off")],
    );
    assert_ne!(h.stdout, "", "passed through unchanged");
    assert_ne!(h.command(), "safesh run --id abc --pending def");
}

#[test]
fn budget_cuts_a_slow_popup() {
    let t = T::new("1\n");
    let start = Instant::now();
    let h = t.hook_with(
        &["hook", "claude", "--budget-sec", "1"],
        TOOL,
        "s1",
        &[("FAKE_DELAY", "5")],
    );
    assert_text_menu(&h);
    assert!(
        start.elapsed() < Duration::from_secs(3),
        "{:?}",
        start.elapsed()
    );
}

// Claude Code ran parallel hooks one after another in a live test, so this checks the queue directly.
#[test]
fn parallel_hooks_show_one_popup_at_a_time() {
    let t = T::new("1\n");
    let slow = [("FAKE_DELAY", "0.5")];
    let a = t.spawn_hook(&["hook", "claude"], TOOL, "s1", &slow);
    let b = t.spawn_hook(&["hook", "claude"], TOOL, "s1", &slow);
    let (a, b) = (wait_hook(a), wait_hook(b));
    let (pa, pb) = (
        arg_of(&a.command(), "--pending"),
        arg_of(&b.command(), "--pending"),
    );
    assert_ne!(pa, pb);
    let log = t.log();
    let popups: Vec<&str> = log.lines().filter(|l| *l != "close").collect();
    assert_eq!(popups, ["start", "end", "start", "end"], "log: {log}");
}

#[test]
fn default_budget_still_asks() {
    let t = T::new("1\n");
    let h = t.hook_with(&["hook", "claude"], TOOL, "s1", &[]);
    assert!(
        h.command().contains("--pending "),
        "{} {}",
        h.stdout,
        h.stderr
    );
    assert!(t.log().contains("start"));
}
