//! End to end tests for `safesh approve-ask` with fake channel binaries.
//! Channel binaries come only from `--bin-dir`, so no real popup or dialog ever opens.
//!
//! Unix only: the fakes are `#!/bin/sh` scripts, and tmux is a Unix channel. The Windows dialog
//! has unit tests in `approval::gui`.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

const FAKE_TMUX: &str = r#"#!/bin/sh
if [ "$1" = "display-message" ]; then echo 1 3.8; exit 0; fi
for a in "$@"; do
  if [ "$a" = "-C" ]; then echo close >> "$FAKE_LOG"; exit 0; fi
done
for last; do :; done
echo "start $FAKE_ID" >> "$FAKE_LOG"
[ -n "$FAKE_DELAY" ] && sleep "$FAKE_DELAY"
if [ "$FAKE_FORGE" = 1 ]; then
  echo '{"nonce":"bad","choice":"global"}' > "$SAFESH_STATE_DIR/approvals-ui/$FAKE_ID.answer.json"
else
  sh -c "$last" < "$FAKE_INPUT"
fi
echo "end $FAKE_ID" >> "$FAKE_LOG"
"#;

const FAKE_DIALOG: &str = "#!/bin/sh\nprintf '%s' \"$FAKE_OUT\"\nexit \"${FAKE_EXIT:-0}\"\n";

struct T {
    td: tempfile::TempDir,
    bin: PathBuf,
    state: PathBuf,
    log: PathBuf,
    input: PathBuf,
}

fn script(dir: &Path, name: &str, body: &str) {
    let p = dir.join(name);
    fs::write(&p, body).unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
}

impl T {
    fn new(input: &str) -> Self {
        let td = tempfile::tempdir().unwrap();
        let bin = td.path().join("bin");
        fs::create_dir(&bin).unwrap();
        script(&bin, "tmux", FAKE_TMUX);
        script(&bin, "osascript", FAKE_DIALOG);
        script(&bin, "zenity", FAKE_DIALOG);
        let state = td.path().join("state");
        let input_f = td.path().join("input");
        fs::write(&input_f, input).unwrap();
        Self {
            log: td.path().join("log"),
            bin,
            state,
            input: input_f,
            td,
        }
    }

    fn cmd(&self, id: &str, extra: &[&str]) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_safesh"));
        c.args(["approve-ask", "--id", id, "--bin-dir"])
            .arg(&self.bin)
            .args(extra)
            .env_remove("SAFESH_APPROVAL")
            .env_remove("SSH_CONNECTION")
            .env_remove("SSH_CLIENT")
            .env_remove("SSH_TTY")
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("DISPLAY")
            .env("HOME", self.td.path())
            .env("SAFESH_STATE_DIR", &self.state)
            .env("FAKE_LOG", &self.log)
            .env("FAKE_INPUT", &self.input)
            .env("FAKE_ID", id)
            .stdin(Stdio::null());
        c
    }

    /// A tmux-channel command.
    fn tmux(&self, id: &str, extra: &[&str]) -> Command {
        let mut c = self.cmd(id, &["--channels", "tmux", "--command", "foo"]);
        c.args(extra)
            .env("TMUX", "/fake,1,0")
            .env("TMUX_PANE", "%1");
        c
    }

    fn log(&self) -> String {
        fs::read_to_string(&self.log).unwrap_or_default()
    }
}

fn out(c: &mut Command) -> String {
    let o = c.output().unwrap();
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

fn spawn(c: &mut Command) -> Child {
    c.stdout(Stdio::piped()).spawn().unwrap()
}

fn finish(c: Child) -> String {
    let o = c.wait_with_output().unwrap();
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

fn ui_dir(t: &T) -> PathBuf {
    t.state.join("approvals-ui")
}

#[test]
fn tmux_command_round_trip() {
    let t = T::new("2\n");
    assert_eq!(out(&mut t.tmux("c1", &[])), "Project");
    fs::write(&t.input, "5\n").unwrap();
    assert_eq!(out(&mut t.tmux("c2", &[])), "Global");
}

#[test]
fn tmux_path_round_trip() {
    let t = T::new("2\n");
    let mut c = t.cmd("p1", &["--channels", "tmux", "--path", "/x/y"]);
    c.env("TMUX", "/fake,1,0").env("TMUX_PANE", "%1");
    assert_eq!(out(&mut c), "Session");
    // Paths offer only 4 choices, so 5 is invalid and EOF follows.
    fs::write(&t.input, "5\n").unwrap();
    let mut c = t.cmd("p2", &["--channels", "tmux", "--path", "/x/y", "--write"]);
    c.env("TMUX", "/fake,1,0").env("TMUX_PANE", "%1");
    assert_eq!(out(&mut c), "none");
}

#[test]
fn invalid_then_valid() {
    let t = T::new("9\nabc\n1\n");
    assert_eq!(out(&mut t.tmux("i1", &[])), "Once");
}

#[test]
fn eof_gives_none() {
    let t = T::new("");
    assert_eq!(out(&mut t.tmux("e1", &[])), "none");
}

#[test]
fn show_timeout_gives_none() {
    let t = T::new("");
    let fifo = t.td.path().join("fifo");
    assert!(
        Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    // Hold the write end open so approve-ui blocks on stdin until its deadline.
    let w = fifo.clone();
    std::thread::spawn(move || {
        let _f = fs::OpenOptions::new().write(true).open(w);
        sleep(Duration::from_secs(8));
    });
    let start = Instant::now();
    let mut c = t.tmux("s1", &["--show-sec", "1"]);
    c.env("FAKE_INPUT", &fifo);
    assert_eq!(out(&mut c), "none");
    let el = start.elapsed();
    assert!(el < Duration::from_secs(4), "took {el:?}");
    // Either approve-ui hit its own deadline, or the popup was closed explicitly.
    let log = t.log();
    assert!(
        log.contains("end s1") || log.contains("close"),
        "log: {log}"
    );
}

#[test]
fn forged_answer_is_rejected() {
    let t = T::new("");
    let mut c = t.tmux("f1", &[]);
    c.env("FAKE_FORGE", "1");
    assert_eq!(out(&mut c), "none");
}

#[test]
fn files_cleaned_and_dir_private() {
    let t = T::new("1\n");
    assert_eq!(out(&mut t.tmux("k1", &[])), "Once");
    let d = ui_dir(&t);
    assert!(!d.join("k1.prompt.json").exists());
    assert!(!d.join("k1.answer.json").exists());
    let mode = fs::metadata(&d).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700);
}

/// Waits until `id` has a queue ticket, so a slow process start can't reorder arrivals.
fn wait_ticket(t: &T, id: &str) {
    let q = ui_dir(t).join("queue");
    let until = Instant::now() + Duration::from_secs(5);
    while !fs::read_dir(&q).is_ok_and(|d| {
        d.flatten()
            .any(|e| e.file_name().to_string_lossy().ends_with(&format!("-{id}")))
    }) && Instant::now() < until
    {
        sleep(Duration::from_millis(10));
    }
}

#[test]
fn queue_runs_prompts_in_order() {
    let t = T::new("1\n");
    let mut kids = vec![];
    for id in ["q1", "q2", "q3"] {
        let mut c = t.tmux(id, &[]);
        c.env("FAKE_DELAY", "0.5");
        kids.push(spawn(&mut c));
        wait_ticket(&t, id);
        sleep(Duration::from_millis(150));
    }
    for k in kids {
        assert_eq!(finish(k), "Once");
    }
    let log = t.log();
    let lines: Vec<&str> = log.lines().collect();
    let want = [
        "start q1", "end q1", "start q2", "end q2", "start q3", "end q3",
    ];
    assert_eq!(lines, want, "log: {log}");
}

#[test]
fn queue_give_up_while_waiting() {
    let t = T::new("1\n");
    let mut a = t.tmux("ga", &["--give-up-sec", "10"]);
    a.env("FAKE_DELAY", "3");
    let a = spawn(&mut a);
    wait_ticket(&t, "ga");
    sleep(Duration::from_millis(200));
    let start = Instant::now();
    let mut b = t.tmux("gb", &["--give-up-sec", "1"]);
    assert_eq!(out(&mut b), "none");
    let el = start.elapsed();
    assert!(el < Duration::from_millis(2500), "took {el:?}");
    assert_eq!(finish(a), "Once");
}

#[test]
fn stale_ticket_is_skipped() {
    let t = T::new("1\n");
    let q = ui_dir(&t).join("queue");
    fs::create_dir_all(&q).unwrap();
    let stale = q.join("000000000000000000000001-999999-stale");
    fs::create_dir(&stale).unwrap();
    fs::set_permissions(&stale, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(out(&mut t.tmux("st1", &[])), "Once");
}

#[test]
fn approval_off_skips_channels() {
    let t = T::new("1\n");
    let mut c = t.tmux("o1", &[]);
    c.env("SAFESH_APPROVAL", "off");
    assert_eq!(out(&mut c), "none");
    assert_eq!(t.log(), "");
}

#[test]
fn no_channel_gives_none() {
    let t = T::new("1\n");
    let mut c = t.cmd("n1", &["--channels", "tmux", "--command", "foo"]);
    assert_eq!(out(&mut c), "none");
    assert_eq!(t.log(), "");
}

fn gui(t: &T, id: &str) -> Command {
    let mut c = t.cmd(id, &["--channels", "gui", "--command", "foo"]);
    if cfg!(target_os = "linux") {
        c.env("DISPLAY", ":0");
    }
    c
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn gui_dialog() {
    let t = T::new("");
    let mut c = gui(&t, "g1");
    c.env("FAKE_OUT", "1. Allow once");
    assert_eq!(out(&mut c), "Once");

    let mut c = gui(&t, "g2");
    c.env("FAKE_OUT", "1. Allow once").env("FAKE_EXIT", "1");
    assert_eq!(out(&mut c), "none");

    #[cfg(target_os = "macos")]
    {
        let mut c = gui(&t, "g3");
        c.env("FAKE_OUT", "false");
        assert_eq!(out(&mut c), "Deny");
    }
}
