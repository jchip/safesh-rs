//! tmux popup over the agent's pane, running `safesh approve-ui`.

use std::path::Path;
use std::process::Command;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use super::{Channel, Choice, Env, Prompt, files, proc, ui};
use crate::hook::sh_quote;

pub struct Tmux;

impl Channel for Tmux {
    fn available(&self, env: &Env) -> bool {
        env.var("TMUX").is_some()
            && env.var("TMUX_PANE").is_some()
            && env.find_bin("tmux").is_some()
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
