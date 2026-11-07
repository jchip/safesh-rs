//! Ask the user directly for an approval: tmux popup or desktop dialog.
//!
//! A channel either returns the user's choice or `None`. `None` means the caller falls back to
//! the text menu. It never means allow.

mod files;
mod gui;
pub mod once;
mod proc;
mod tmux;
pub mod ui;

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
        commands: Vec<String>,
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
        Request::Command { commands } => commands.clone(),
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
    /// Path to this safesh binary, for the tmux popup.
    pub exe: PathBuf,
    pub state_dir: PathBuf,
    /// Where `tmux`, `osascript`, `zenity` and `kdialog` are looked up. Never `$PATH`: at
    /// runtime the env comes from the agent's shell, which could put a fake dialog first.
    pub bin_dirs: Vec<PathBuf>,
}

impl Env {
    const KEYS: &[&str] = &[
        "SAFESH_APPROVAL",
        "TMUX",
        "TMUX_PANE",
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "SSH_CONNECTION",
        "SSH_TTY",
    ];

    const BIN_DIRS: &[&str] = &["/usr/bin", "/bin", "/opt/homebrew/bin", "/usr/local/bin"];

    #[must_use]
    pub fn from_process(state_dir: &Path) -> Self {
        let vars = Self::KEYS
            .iter()
            .filter_map(|k| {
                std::env::var(k)
                    .ok()
                    .filter(|v| !v.is_empty())
                    .map(|v| ((*k).to_string(), v))
            })
            .collect();
        Self {
            vars,
            macos: cfg!(target_os = "macos"),
            exe: std::env::current_exe().unwrap_or_else(|_| PathBuf::from("safesh")),
            state_dir: state_dir.to_path_buf(),
            bin_dirs: Self::bin_dirs(),
        }
    }

    /// Debug builds let e2e tests point at fake channel binaries. Release builds never do.
    fn bin_dirs() -> Vec<PathBuf> {
        #[cfg(debug_assertions)]
        if let Some(d) = std::env::var_os("SAFESH_TEST_BIN_DIRS") {
            return std::env::split_paths(&d).collect();
        }
        Self::BIN_DIRS.iter().map(PathBuf::from).collect()
    }

    /// First executable file called `name` in `bin_dirs`.
    #[must_use]
    pub fn find_bin(&self, name: &str) -> Option<PathBuf> {
        use std::os::unix::fs::PermissionsExt;
        self.bin_dirs.iter().map(|d| d.join(name)).find(|p| {
            p.metadata()
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
    }

    #[must_use]
    pub fn var(&self, k: &str) -> Option<&str> {
        self.vars.get(k).map(String::as_str)
    }

    fn over_ssh(&self) -> bool {
        self.var("SSH_CONNECTION").is_some() || self.var("SSH_TTY").is_some()
    }
}

pub trait Channel {
    fn available(&self, env: &Env) -> bool;
    /// `None` means no answer: timeout, closed window, or channel error.
    fn ask(&self, env: &Env, prompt: &Prompt, deadline: Instant) -> Option<Choice>;
}

/// Popup channels by config name. `ask` is not here: it is hook output, not a popup.
fn channel(name: &str) -> Option<Box<dyn Channel>> {
    match name {
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
    let ch = channels.iter().find(|c| c.available(env))?;
    let _turn = files::Queue::wait_turn(&env.state_dir, &prompt.id, timing.give_up_at)?;
    let deadline = (Instant::now() + timing.show).min(timing.give_up_at);
    ch.ask(env, prompt, deadline)
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
