//! Desktop dialog channel: `osascript` on macOS, `zenity` or `kdialog` on Linux.
//!
//! Prompt text and labels reach the dialog only as argv items, never as script source.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use super::{Channel, Choice, Env, Prompt, body_lines, choice_from_label, numbered_labels, proc};

pub struct Gui;

impl Channel for Gui {
    fn available(&self, env: &Env) -> bool {
        if tmux_client_over_ssh(env) {
            return false;
        }
        if env.macos {
            return !env.over_ssh() && env.find_bin("osascript").is_some();
        }
        let display = env.var("DISPLAY").is_some() || env.var("WAYLAND_DISPLAY").is_some();
        display && (env.find_bin("zenity").is_some() || env.find_bin("kdialog").is_some())
    }

    fn ask(&self, env: &Env, prompt: &Prompt, deadline: Instant) -> Option<Choice> {
        if env.macos {
            let out = proc::run(osascript_cmd(&env.find_bin("osascript")?, prompt), deadline)?;
            return match out.trim() {
                "false" => Some(Choice::Deny),
                s => choice_from_label(&prompt.request, s),
            };
        }
        let cmd = match env.find_bin("zenity") {
            Some(z) => zenity_cmd(&z, prompt),
            None => kdialog_cmd(&env.find_bin("kdialog")?, prompt),
        };
        choice_from_label(&prompt.request, &proc::run(cmd, deadline)?)
    }
}

/// Inside tmux our SSH vars can be stale. tmux copies `SSH_CONNECTION` from each attaching
/// client into the session env (default `update-environment`), so ask tmux instead.
/// Any tmux error keeps the env-var checks as the only signal.
fn tmux_client_over_ssh(env: &Env) -> bool {
    let (Some(_), Some(tmux)) = (env.var("TMUX"), env.find_bin("tmux")) else {
        return false;
    };
    let mut cmd = Command::new(tmux);
    cmd.arg("show-environment");
    if let Some(pane) = env.var("TMUX_PANE") {
        cmd.args(["-t", pane]);
    }
    cmd.arg("SSH_CONNECTION");
    proc::run(cmd, Instant::now() + Duration::from_millis(500))
        .is_some_and(|out| out.starts_with("SSH_CONNECTION="))
}

fn text(prompt: &Prompt) -> String {
    format!(
        "{}\n\n{}",
        prompt.request.title(),
        body_lines(prompt).join("\n")
    )
}

fn deny_index(prompt: &Prompt) -> usize {
    let choices = prompt.request.choices();
    choices.iter().position(|c| *c == Choice::Deny).unwrap_or(0)
}

/// The script is fixed. argv: prompt text, default label (Deny), then all labels.
fn osascript_cmd(osascript: &Path, prompt: &Prompt) -> Command {
    let labels = numbered_labels(&prompt.request);
    let mut cmd = Command::new(osascript);
    cmd.args([
        "-e",
        "on run argv",
        "-e",
        "set r to choose from list (items 3 thru -1 of argv) with title \"safesh\" \
         with prompt (item 1 of argv) default items {item 2 of argv}",
        "-e",
        "if r is false then return \"false\"",
        "-e",
        "return item 1 of r",
        "-e",
        "end run",
    ]);
    cmd.arg(text(prompt))
        .arg(&labels[deny_index(prompt)])
        .args(&labels);
    cmd
}

fn zenity_cmd(zenity: &Path, prompt: &Prompt) -> Command {
    let mut cmd = Command::new(zenity);
    cmd.args([
        "--list",
        "--radiolist",
        "--no-markup",
        "--hide-header",
        "--title=safesh",
    ])
    .arg(format!("--text={}", text(prompt)))
    .args(["--column=Pick", "--column=Choice"]);
    let deny = deny_index(prompt);
    for (i, label) in numbered_labels(&prompt.request).iter().enumerate() {
        cmd.arg(if i == deny { "TRUE" } else { "FALSE" }).arg(label);
    }
    cmd
}

fn kdialog_cmd(kdialog: &Path, prompt: &Prompt) -> Command {
    let mut cmd = Command::new(kdialog);
    cmd.args(["--title", "safesh", "--radiolist"])
        .arg(text(prompt));
    let deny = deny_index(prompt);
    for (i, label) in numbered_labels(&prompt.request).iter().enumerate() {
        cmd.arg(label)
            .arg(label)
            .arg(if i == deny { "on" } else { "off" });
    }
    cmd
}

#[cfg(all(test, unix))]
mod tests {
    use std::collections::HashMap;

    use safesh_policy::state::pending::PathOperation;

    use super::super::Request;
    use super::*;

    fn cmd_prompt(c: &str) -> Prompt {
        Prompt {
            id: "p1".into(),
            cwd: "/work".into(),
            request: Request::Command {
                commands: vec![c.into()],
            },
        }
    }

    fn path_prompt() -> Prompt {
        Prompt {
            id: "p2".into(),
            cwd: "/work".into(),
            request: Request::Path {
                path: "/etc/x".into(),
                op: PathOperation::Read,
                command: Some("cat /etc/x".into()),
            },
        }
    }

    fn args(c: &Command) -> Vec<String> {
        c.get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    fn script_args(a: &[String]) -> Vec<&String> {
        a.iter()
            .enumerate()
            .filter(|(i, _)| *i > 0 && a[i - 1] == "-e")
            .map(|(_, s)| s)
            .collect()
    }

    #[test]
    fn osascript_text_and_labels_are_argv_items() {
        for p in [cmd_prompt("ls -la"), path_prompt()] {
            let c = osascript_cmd(Path::new("/usr/bin/osascript"), &p);
            assert_eq!(c.get_program(), "/usr/bin/osascript");
            let a = args(&c);
            let scripts = script_args(&a);
            let t = text(&p);
            assert!(
                scripts
                    .iter()
                    .all(|s| !s.contains(&t) && !s.contains("/etc/x"))
            );
            let tail = &a[a.len() - 2 - p.request.choices().len()..];
            assert_eq!(tail[0], t);
            assert_eq!(tail[1], deny_label(&p));
            assert_eq!(&tail[2..], numbered_labels(&p.request).as_slice());
        }
    }

    fn deny_label(p: &Prompt) -> String {
        numbered_labels(&p.request)[deny_index(p)].clone()
    }

    #[test]
    fn hostile_text_only_in_argv() {
        let evil = "\"; do shell script \"rm -rf ~\"\nend run\n\\ \\\" x";
        let p = cmd_prompt(evil);
        let a = args(&osascript_cmd(Path::new("/usr/bin/osascript"), &p));
        for s in script_args(&a) {
            assert!(!s.contains("rm -rf") && !s.contains("shell script"));
        }
        assert!(a.iter().any(|s| s.contains("rm -rf")));
    }

    #[test]
    fn output_mapping() {
        for p in [cmd_prompt("ls"), path_prompt()] {
            let r = &p.request;
            for (c, l) in r.choices().iter().zip(numbered_labels(r)) {
                assert_eq!(choice_from_label(r, &format!("{l}\n")), Some(*c));
            }
            assert_eq!(choice_from_label(r, "garbage"), None);
            assert_eq!(
                r.choices().len(),
                if matches!(r, Request::Path { .. }) {
                    4
                } else {
                    5
                }
            );
        }
    }

    #[test]
    fn zenity_args() {
        let p = cmd_prompt("ls");
        let c = zenity_cmd(Path::new("/usr/bin/zenity"), &p);
        let a = args(&c);
        assert!(a.contains(&"--no-markup".to_string()));
        let labels = numbered_labels(&p.request);
        for (i, l) in labels.iter().enumerate() {
            let pos = a.iter().position(|s| s == l).unwrap();
            assert_eq!(
                a[pos - 1],
                if i == deny_index(&p) { "TRUE" } else { "FALSE" }
            );
        }
    }

    #[test]
    fn kdialog_args() {
        let p = path_prompt();
        let a = args(&kdialog_cmd(Path::new("/usr/bin/kdialog"), &p));
        for (i, l) in numbered_labels(&p.request).iter().enumerate() {
            let pos = a.iter().position(|s| s == l).unwrap();
            assert_eq!(&a[pos + 1], l);
            assert_eq!(a[pos + 2], if i == deny_index(&p) { "on" } else { "off" });
        }
    }

    fn env(macos: bool, vars: &[(&str, &str)], bins: &[&str]) -> (Env, tempfile::TempDir) {
        let bins: Vec<(&str, &str)> = bins.iter().map(|b| (*b, "")).collect();
        env_with(macos, vars, &bins)
    }

    /// Like `env`, with each bin given its file content.
    fn env_with(
        macos: bool,
        vars: &[(&str, &str)],
        bins: &[(&str, &str)],
    ) -> (Env, tempfile::TempDir) {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        for (b, body) in bins {
            let p = dir.path().join(b);
            std::fs::write(&p, body).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| ((*k).into(), (*v).into()))
            .collect();
        let bin_dirs = vec![dir.path().to_path_buf()];
        (
            Env {
                vars,
                macos,
                bin_dirs,
                ..Env::default()
            },
            dir,
        )
    }

    #[test]
    fn available_by_platform() {
        let ok =
            |macos, vars: &[(&str, &str)], bins: &[&str]| Gui.available(&env(macos, vars, bins).0);
        assert!(ok(true, &[], &["osascript"]));
        assert!(!ok(true, &[], &[]));
        assert!(!ok(true, &[("SSH_CONNECTION", "1 2 3 4")], &["osascript"]));
        assert!(!ok(false, &[], &["zenity"]));
        assert!(ok(false, &[("DISPLAY", ":0")], &["zenity"]));
        assert!(ok(false, &[("WAYLAND_DISPLAY", "w")], &["kdialog"]));
        assert!(!ok(false, &[("DISPLAY", ":0")], &[]));
    }

    #[test]
    fn tmux_client_over_ssh_blocks_gui() {
        let tmux = |out: &str| format!("#!/bin/sh\nprintf '%s\\n' '{out}'\n");
        let in_tmux = [("TMUX", "/tmp/tmux-1/default,1,0"), ("TMUX_PANE", "%1")];
        let ok = |macos, vars: &[(&str, &str)], tmux_out: &str| {
            let dialog = if macos { "osascript" } else { "zenity" };
            let t = tmux(tmux_out);
            Gui.available(&env_with(macos, vars, &[(dialog, ""), ("tmux", &t)]).0)
        };
        let ssh = "SSH_CONNECTION=1.2.3.4 5 6.7.8.9 22";
        assert!(!ok(true, &in_tmux, ssh));
        assert!(ok(true, &in_tmux, "-SSH_CONNECTION"));
        // Not inside tmux: tmux is never asked.
        assert!(ok(true, &[], ssh));
        let linux = [in_tmux[0], in_tmux[1], ("DISPLAY", ":0")];
        assert!(!ok(false, &linux, ssh));
        assert!(ok(false, &linux, "-SSH_CONNECTION"));
        // A failing tmux leaves the env-var checks in charge.
        let fail = "#!/bin/sh\nexit 1\n";
        let e = env_with(true, &in_tmux, &[("osascript", ""), ("tmux", fail)]);
        assert!(Gui.available(&e.0));
    }
}
