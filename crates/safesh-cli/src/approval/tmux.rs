//! tmux popup over the agent's pane, running `safesh approve-ui`.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::{Channel, Choice, Env, Prompt, files, proc, ui};
use crate::hook::sh_quote;

pub struct Tmux;

impl Channel for Tmux {
    fn name(&self) -> &'static str {
        "tmux"
    }

    fn available(&self, env: &Env) -> bool {
        let (Some(_), Some(pane), Some(tmux)) =
            (env.var("TMUX"), env.var("TMUX_PANE"), env.find_bin("tmux"))
        else {
            return false;
        };
        popup_can_show(&tmux, pane)
    }

    fn ask(&self, env: &Env, prompt: &Prompt, deadline: Instant) -> Option<Choice> {
        let pane = env.var("TMUX_PANE")?;
        let tmux = env.find_bin("tmux")?;
        let nonce = files::write_prompt(&env.state_dir, prompt)?;
        let shown = proc::run(popup_cmd(&tmux, env, prompt, pane, deadline), deadline);
        if shown.is_none() {
            // Killing the tmux client leaves the popup up, so close it explicitly.
            let _ = Command::new(&tmux)
                .args(["display-popup", "-C", "-t", pane])
                .status();
        }
        files::take_answer(&env.state_dir, &prompt.id, &nonce)
    }
}

/// The pane still exists, a client has its session attached, and tmux has `display-popup`.
fn popup_can_show(tmux: &Path, pane: &str) -> bool {
    let mut cmd = Command::new(tmux);
    cmd.args([
        "display-message",
        "-p",
        "-t",
        pane,
        "#{session_attached} #{version}",
    ]);
    // tmux answers in milliseconds. The slack covers a slow first exec.
    proc::run(cmd, Instant::now() + Duration::from_secs(2)).is_some_and(|out| {
        let mut it = out.split_whitespace();
        let attached = it.next().and_then(|n| n.parse::<u32>().ok());
        attached.is_some_and(|n| n > 0) && it.next().is_some_and(has_popup)
    })
}

/// The agent's pane as `session:window (name)`. The popup shows on whatever window the user is
/// viewing, so this says which agent asks.
pub fn location(env: &Env) -> Option<String> {
    let (Some(_), Some(pane), Some(tmux)) =
        (env.var("TMUX"), env.var("TMUX_PANE"), env.find_bin("tmux"))
    else {
        return None;
    };
    let mut cmd = Command::new(tmux);
    cmd.args([
        "display-message",
        "-p",
        "-t",
        pane,
        "#{session_name}:#{window_index} (#{window_name})",
    ]);
    let out = proc::run(cmd, Instant::now() + Duration::from_secs(2))?;
    let out = out.trim();
    (!out.is_empty()).then(|| out.to_string())
}

/// `display-popup` came in tmux 3.2. Versions look like `3.4`, `3.3a`, `next-3.6` or
/// `openbsd-7.4`.
fn has_popup(version: &str) -> bool {
    let v = version.rsplit('-').next().unwrap_or(version);
    let mut parts = v.split('.');
    let num = |s: Option<&str>| {
        let d: String = s?.chars().take_while(char::is_ascii_digit).collect();
        d.parse::<u32>().ok()
    };
    match (num(parts.next()), num(parts.next())) {
        (Some(major), Some(minor)) => (major, minor) >= (3, 2),
        _ => false,
    }
}

fn popup_cmd(tmux: &Path, env: &Env, prompt: &Prompt, pane: &str, deadline: Instant) -> Command {
    let left = deadline.saturating_duration_since(Instant::now());
    let deadline_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .saturating_add(left)
        .as_millis();
    // A single shell string: tmux runs it with `sh -c`, so every part is quoted.
    let inner = [
        env.exe.to_string_lossy().as_ref(),
        "approve-ui",
        "--id",
        &prompt.id,
        "--state-dir",
        env.state_dir.to_string_lossy().as_ref(),
        "--deadline-ms",
        &deadline_ms.to_string(),
    ]
    .map(sh_quote)
    .join(" ");
    let mut cmd = Command::new(tmux);
    cmd.args(["display-popup", "-E", "-t", pane, "-w"])
        .arg(ui::POPUP_WIDTH.to_string())
        .arg("-h")
        .arg(ui::popup_rows(prompt).to_string())
        .arg("-T")
        .arg(format!(" {} ", prompt.request.title()))
        .arg(&inner);
    cmd
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    /// Env inside tmux pane %1, with a fake `tmux` that runs `script`.
    fn env(script: &str) -> (Env, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("tmux");
        std::fs::write(&p, script).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut env = Env {
            bin_dirs: vec![dir.path().to_path_buf()],
            ..Env::default()
        };
        env.vars
            .insert("TMUX".into(), "/tmp/tmux-1/default,1,0".into());
        env.vars.insert("TMUX_PANE".into(), "%1".into());
        (env, dir)
    }

    #[test]
    fn popup_versions() {
        for v in ["3.2", "3.3a", "3.10", "4.0", "next-3.6", "openbsd-7.4"] {
            assert!(has_popup(v), "{v}");
        }
        for v in ["3.1", "3.1c", "2.9a", "next", "", "x.y"] {
            assert!(!has_popup(v), "{v}");
        }
    }

    #[test]
    fn location_names_session_and_window() {
        let loc = |script: &str| location(&env(script).0);
        assert_eq!(
            loc("#!/bin/sh\necho 'work:2 (claude)'\n").as_deref(),
            Some("work:2 (claude)")
        );
        assert_eq!(loc("#!/bin/sh\nexit 1\n"), None);
        assert_eq!(loc("#!/bin/sh\necho\n"), None);
        let (mut e, _d) = env("#!/bin/sh\necho 'work:2 (claude)'\n");
        e.vars.remove("TMUX_PANE");
        assert_eq!(location(&e), None);
    }

    #[test]
    fn available_only_when_pane_is_attached() {
        let ok = |script: &str| Tmux.available(&env(script).0);
        assert!(ok("#!/bin/sh\necho 1 3.8\n"));
        assert!(ok("#!/bin/sh\necho 2 3.2\n"));
        // Detached session.
        assert!(!ok("#!/bin/sh\necho 0 3.8\n"));
        // No display-popup before 3.2, and no version means unknown.
        assert!(!ok("#!/bin/sh\necho 1 3.1c\n"));
        assert!(!ok("#!/bin/sh\necho 1\n"));
        // Pane gone: tmux errors.
        assert!(!ok("#!/bin/sh\necho \"can't find pane\" >&2; exit 1\n"));
        // Not inside tmux.
        let (mut e, _d) = env("#!/bin/sh\necho 1 3.8\n");
        e.vars.remove("TMUX");
        assert!(!Tmux.available(&e));
    }
}
