//! Ask the user directly for an approval: tmux popup or desktop dialog (osascript, zenity or
//! kdialog, or a Windows Forms dialog run by `powershell.exe` on Windows).
//!
//! A channel either returns the user's choice or `None`. `None` means the caller falls back to
//! the text menu. It never means allow.

mod files;
mod gui;
pub mod once;
mod proc;
mod tmux;
pub mod toa;
pub mod ui;

#[allow(unused_imports)]
pub use toa::{
    ApprovalOutcome, CommandApprovalRequest, PathAccessMode, PathApprovalOutcome,
    PathApprovalRequest, ToaChannel, resolve_approval, resolve_path_approval,
};

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use safesh_policy::state::pending::PathOperation;
use serde::{Deserialize, Serialize};

/// What the user is asked to approve.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Request {
    Command {
        /// Command names to grant.
        commands: Vec<String>,
        /// The agent's full command line, shown for context.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        command: Option<String>,
    },
    Path {
        path: String,
        op: PathOperation,
        /// The agent's command that touches the path, shown for context.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        command: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Prompt {
    /// Pending id. Also names the prompt and answer files, so it must pass `check_id`.
    pub id: String,
    pub cwd: String,
    pub request: Request,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Choice {
    Once,
    Project,
    Session,
    Global,
    Deny,
}

impl Request {
    /// Choices offered, in display order. Paths have no global scope, like `retry-path`.
    #[must_use]
    pub fn choices(&self) -> &'static [Choice] {
        match self {
            Self::Command { .. } => &[
                Choice::Once,
                Choice::Project,
                Choice::Session,
                Choice::Deny,
                Choice::Global,
            ],
            Self::Path { .. } => &[Choice::Once, Choice::Session, Choice::Project, Choice::Deny],
        }
    }

    /// Fixed one-line title. Never built from agent text.
    #[must_use]
    pub fn title(&self) -> &'static str {
        match self {
            Self::Command { .. } => "safesh: allow these commands?",
            Self::Path {
                op: PathOperation::Read,
                ..
            } => "safesh: allow reading this path?",
            Self::Path { .. } => "safesh: allow writing this path?",
        }
    }
}

impl Choice {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Once => "Allow once",
            Self::Project => "Always allow (this project)",
            Self::Session => "Allow for session",
            Self::Deny => "Deny",
            Self::Global => "Always allow (all projects)",
        }
    }
}

/// Numbered labels as shown in every channel: `"1. Allow once"`.
#[must_use]
pub fn numbered_labels(req: &Request) -> Vec<String> {
    req.choices()
        .iter()
        .enumerate()
        .map(|(i, c)| format!("{}. {}", i + 1, c.label()))
        .collect()
}

/// Maps a picked label (as a channel returns it) back to a choice.
#[must_use]
pub fn choice_from_label(req: &Request, label: &str) -> Option<Choice> {
    let label = label.trim();
    numbered_labels(req)
        .iter()
        .position(|l| l == label)
        .map(|i| req.choices()[i])
}

/// Lines with at most this many characters, and at most `MAX_LINES` of them, are shown.
const MAX_LINE: usize = 200;
const MAX_LINES: usize = 8;

/// Body text: the agent's commands, or the path and its command. Plain and truncated.
#[must_use]
pub fn body_lines(prompt: &Prompt) -> Vec<String> {
    let items: Vec<String> = match &prompt.request {
        Request::Command { commands, command } => {
            let mut v = commands.clone();
            v.extend(command.iter().map(|c| format!("command: {c}")));
            v
        }
        Request::Path { path, command, .. } => {
            let mut v = vec![format!("path: {path}")];
            v.extend(command.iter().map(|c| format!("command: {c}")));
            v
        }
    };
    let mut out: Vec<String> = items
        .iter()
        .flat_map(|s| s.lines().map(str::to_string).collect::<Vec<_>>())
        .map(|l| clip(&l))
        .collect();
    if out.len() > MAX_LINES {
        let more = out.len() - (MAX_LINES - 1);
        out.truncate(MAX_LINES - 1);
        out.push(format!("+{more} more"));
    }
    out.push(String::new());
    out.push(format!("in {}", clip(&prompt.cwd)));
    out
}

fn clip(s: &str) -> String {
    let s: String = s.chars().filter(|c| !c.is_control()).collect();
    if s.chars().count() <= MAX_LINE {
        return s;
    }
    let mut t: String = s.chars().take(MAX_LINE - 1).collect();
    t.push('…');
    t
}

/// The env vars channels look at, captured once.
#[derive(Debug, Clone, Default)]
pub struct Env {
    pub vars: HashMap<String, String>,
    pub macos: bool,
    pub windows: bool,
    /// Windows dir (`C:\Windows`), from Win32. The dialog child gets it as `SystemRoot`.
    pub win_root: Option<PathBuf>,
    /// Path to this safesh binary, for the tmux popup.
    pub exe: PathBuf,
    pub state_dir: PathBuf,
    /// Where `tmux`, `osascript`, `zenity`, `kdialog` and `powershell.exe` are looked up. Never
    /// `$PATH`: at runtime the env comes from the agent's shell, which could put a fake dialog
    /// first. On Windows it is `System32\WindowsPowerShell\v1.0`, with the system dir from Win32.
    pub bin_dirs: Vec<PathBuf>,
    /// A desktop of this user is on screen, so a dialog can be seen. Only the GUI channel reads it.
    pub desktop: bool,
}

impl Env {
    const KEYS: &[&str] = &[
        "SAFESH_APPROVAL",
        "TMUX",
        "TMUX_PANE",
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "XDG_RUNTIME_DIR",
        "SSH_CONNECTION",
        "SSH_CLIENT",
        "SSH_TTY",
        // Windows: `Console` or `RDP-Tcp#N` in a desktop session; unset over SSH and in services.
        "SESSIONNAME",
        // TOA Gateway and Host Engine environment variables.
        "TOA_GATEWAY_URL",
        "TOA_HOOK_TOKEN",
        "TOA_TERMINAL_ID",
        "TOA_API_DISCOVERY",
    ];

    #[cfg(not(windows))]
    const BIN_DIRS: &[&str] = &["/usr/bin", "/bin", "/opt/homebrew/bin", "/usr/local/bin"];

    #[must_use]
    pub fn from_process(state_dir: &Path) -> Self {
        let vars: HashMap<String, String> = Self::KEYS
            .iter()
            .filter_map(|k| {
                std::env::var(k)
                    .ok()
                    .filter(|v| !v.is_empty())
                    .map(|v| ((*k).to_string(), v))
            })
            .collect();
        #[cfg(windows)]
        let win_root =
            safesh_policy::compat::system_dir().and_then(|d| d.parent().map(Path::to_path_buf));
        #[cfg(not(windows))]
        let win_root = None;
        let desktop = gui::desktop_on_screen(&vars);
        Self {
            vars,
            macos: cfg!(target_os = "macos"),
            windows: cfg!(windows),
            win_root,
            exe: std::env::current_exe().unwrap_or_else(|_| PathBuf::from("safesh")),
            state_dir: state_dir.to_path_buf(),
            bin_dirs: Self::bin_dirs(),
            desktop,
        }
    }

    /// Debug builds let e2e tests point at fake channel binaries. Release builds never do.
    fn bin_dirs() -> Vec<PathBuf> {
        #[cfg(debug_assertions)]
        if let Some(d) = std::env::var_os("SAFESH_TEST_BIN_DIRS") {
            return std::env::split_paths(&d).collect();
        }
        #[cfg(windows)]
        return safesh_policy::compat::system_dir()
            .map(|d| d.join("WindowsPowerShell").join("v1.0"))
            .into_iter()
            .collect();
        #[cfg(not(windows))]
        Self::BIN_DIRS.iter().map(PathBuf::from).collect()
    }

    /// First executable file called `name` in `bin_dirs`.
    #[must_use]
    pub fn find_bin(&self, name: &str) -> Option<PathBuf> {
        #[cfg(unix)]
        use std::os::unix::fs::PermissionsExt;
        self.bin_dirs.iter().map(|d| d.join(name)).find(|p| {
            #[cfg(unix)]
            let exec = |m: &std::fs::Metadata| m.permissions().mode() & 0o111 != 0;
            // Windows has no exec bit; the name carries `.exe`.
            #[cfg(not(unix))]
            let exec = |_: &std::fs::Metadata| true;
            p.metadata().is_ok_and(|m| m.is_file() && exec(&m))
        })
    }

    #[must_use]
    pub fn var(&self, k: &str) -> Option<&str> {
        self.vars.get(k).map(String::as_str)
    }

    fn over_ssh(&self) -> bool {
        ["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"]
            .iter()
            .any(|k| self.var(k).is_some())
    }
}

pub trait Channel {
    fn available(&self, env: &Env) -> bool;
    /// `None` means no answer: timeout, closed window, or channel error.
    fn ask(&self, env: &Env, prompt: &Prompt, deadline: Instant) -> Option<Choice>;
    /// Whether this channel requires modal serialization via `files::Queue::wait_turn`.
    /// Desktop dialogs and tmux popups are modal and collide if shown simultaneously.
    /// Non-modal channels such as TOA (`ToaChannel`) manage multiplexed queues centrally
    /// and must not acquire the local machine-wide UI lock.
    fn is_modal(&self) -> bool {
        true
    }
}

/// Approval channels by config name. `ask` is not here: it is hook output, not a channel driver.
fn channel(name: &str) -> Option<Box<dyn Channel>> {
    match name {
        "toa" => Some(Box::new(toa::ToaChannel)),
        "tmux" => Some(Box::new(tmux::Tmux)),
        "gui" => Some(Box::new(gui::Gui)),
        _ => None,
    }
}

/// How long a prompt may take.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    /// Time the user gets once the prompt is on screen.
    pub show: Duration,
    /// Hard stop for queue wait plus showing. The caller sets it to fit its own outer timeout
    /// (hook timeout, or the agent's Bash tool timeout).
    pub give_up_at: Instant,
}

/// Asks the first available channel in `order`. Prompts queue: each one is shown after the
/// one before it is answered, in arrival order.
#[must_use]
pub fn ask_user(order: &[String], env: &Env, prompt: &Prompt, timing: Timing) -> Option<Choice> {
    let channels: Vec<Box<dyn Channel>> = order.iter().filter_map(|n| channel(n)).collect();
    ask_with(&channels, env, prompt, timing)
}

/// Like `ask_user`, with the channels given. Tests pass fakes here.
#[must_use]
pub fn ask_with(
    channels: &[Box<dyn Channel>],
    env: &Env,
    prompt: &Prompt,
    timing: Timing,
) -> Option<Choice> {
    if env.var("SAFESH_APPROVAL") == Some("off") {
        return None;
    }
    // Channels are asked in the configured order. Modal channels (desktop dialog, tmux popup)
    // collide when two are on screen, so the first one reached takes this machine's prompt
    // queue and holds it for the rest of the call. Non-modal channels (TOA) queue centrally
    // and never take it, so several agents can have requests pending there at once.
    let mut turn: Option<files::Queue> = None;
    let mut modal_deadline: Option<Instant> = None;
    for ch in channels.iter().filter(|c| c.available(env)) {
        if Instant::now() >= timing.give_up_at {
            return None;
        }
        let deadline = if ch.is_modal() {
            if turn.is_none() {
                turn = files::Queue::wait_turn(&env.state_dir, &prompt.id, timing.give_up_at);
            }
            if turn.is_none() {
                // The prompt queue stayed busy. A later non-modal channel does not need it.
                continue;
            }
            // Modal channels share one window, counted from when the turn was won.
            *modal_deadline
                .get_or_insert_with(|| (Instant::now() + timing.show).min(timing.give_up_at))
        } else {
            // Do not keep other prompts off the screen while a non-modal channel waits.
            turn = None;
            modal_deadline = None;
            (Instant::now() + timing.show).min(timing.give_up_at)
        };
        if let Some(choice) = ch.ask(env, prompt, deadline) {
            return Some(choice);
        }
    }
    None
}

/// `safesh approve-ask`: shows one prompt and prints the choice. For manual and integration
/// tests only. It grants nothing, so `bin_dirs` may be overridden here.
pub fn cli_ask(
    id: &str,
    request: Request,
    channels: &[String],
    timing: Timing,
    bin_dirs: Vec<PathBuf>,
) -> u8 {
    let host = safesh_policy::host::HostEnv::from_process();
    let mut env = Env::from_process(&host.state_dir);
    if !bin_dirs.is_empty() {
        env.bin_dirs = bin_dirs;
        // Fake dialogs need no real desktop.
        env.desktop = true;
    }
    let prompt = Prompt {
        id: id.to_string(),
        cwd: std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
        request,
    };
    match ask_user(channels, &env, &prompt, timing) {
        Some(c) => println!("{c:?}"),
        None => println!("none"),
    }
    0
}

#[cfg(test)]
mod order_tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct Fake {
        name: &'static str,
        modal: bool,
        answer: Option<Choice>,
        log: Arc<Mutex<Vec<String>>>,
    }

    impl Channel for Fake {
        fn available(&self, _: &Env) -> bool {
            true
        }
        fn is_modal(&self) -> bool {
            self.modal
        }
        fn ask(&self, env: &Env, prompt: &Prompt, _: Instant) -> Option<Choice> {
            // A ticket in the queue dir means this call holds the machine-wide prompt turn.
            let holds_turn = std::fs::read_dir(files::ui_dir(&env.state_dir).join("queue"))
                .is_ok_and(|d| {
                    d.flatten()
                        .any(|e| e.file_name().to_string_lossy().ends_with(&prompt.id))
                });
            self.log
                .lock()
                .unwrap()
                .push(format!("{} turn={holds_turn}", self.name));
            self.answer
        }
    }

    fn run(order: &[(&'static str, bool, Option<Choice>)]) -> (Option<Choice>, Vec<String>) {
        let dir = tempfile::tempdir().unwrap();
        let env = Env {
            state_dir: dir.path().to_path_buf(),
            ..Env::default()
        };
        let log = Arc::new(Mutex::new(Vec::new()));
        let channels: Vec<Box<dyn Channel>> = order
            .iter()
            .map(|&(name, modal, answer)| {
                Box::new(Fake {
                    name,
                    modal,
                    answer,
                    log: Arc::clone(&log),
                }) as Box<dyn Channel>
            })
            .collect();
        let prompt = Prompt {
            id: "order-1".into(),
            cwd: "/".into(),
            request: Request::Command {
                commands: vec!["ls".into()],
                command: None,
            },
        };
        let timing = Timing {
            show: Duration::from_secs(5),
            give_up_at: Instant::now() + Duration::from_secs(10),
        };
        let choice = ask_with(&channels, &env, &prompt, timing);
        let log = log.lock().unwrap().clone();
        (choice, log)
    }

    #[test]
    fn channels_are_asked_in_configured_order() {
        let (choice, log) = run(&[("tmux", true, None), ("toa", false, Some(Choice::Once))]);
        assert_eq!(choice, Some(Choice::Once));
        // The queue is released before the non-modal channel waits.
        assert_eq!(log, ["tmux turn=true", "toa turn=false"]);

        let (choice, log) = run(&[("toa", false, None), ("gui", true, Some(Choice::Deny))]);
        assert_eq!(choice, Some(Choice::Deny));
        // The non-modal channel runs without the machine-wide prompt queue.
        assert_eq!(log, ["toa turn=false", "gui turn=true"]);
    }

    #[test]
    fn first_answer_wins_and_later_channels_are_not_asked() {
        let (choice, log) = run(&[
            ("toa", false, Some(Choice::Session)),
            ("gui", true, Some(Choice::Deny)),
        ]);
        assert_eq!(choice, Some(Choice::Session));
        assert_eq!(log, ["toa turn=false"]);
    }
}
