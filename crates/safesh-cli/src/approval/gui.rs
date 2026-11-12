//! Desktop dialog channel: `osascript` on macOS, `zenity` or `kdialog` on Linux, and a Windows
//! Forms dialog run by `powershell.exe` on Windows.
//!
//! Prompt text and labels reach the dialog only as argv items (env vars on Windows), never as
//! script source.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use super::{Channel, Choice, Env, Prompt, body_lines, choice_from_label, numbered_labels, proc};

pub struct Gui;

impl Channel for Gui {
    fn available(&self, env: &Env) -> bool {
        if env.windows {
            // Over SSH or in a service there is no desktop to show the dialog on.
            return !env.over_ssh()
                && env.var("SESSIONNAME").is_some()
                && env.win_root.is_some()
                && env.find_bin("powershell.exe").is_some();
        }
        if tmux_client_over_ssh(env) {
            return false;
        }
        if env.macos {
            return !env.over_ssh() && env.desktop && env.find_bin("osascript").is_some();
        }
        let display = env.var("DISPLAY").is_some() || env.var("WAYLAND_DISPLAY").is_some();
        display
            && env.desktop
            && (env.find_bin("zenity").is_some() || env.find_bin("kdialog").is_some())
    }

    fn ask(&self, env: &Env, prompt: &Prompt, deadline: Instant) -> Option<Choice> {
        if env.windows {
            let ps = env.find_bin("powershell.exe")?;
            let out = proc::run(windows_cmd(&ps, env.win_root.as_deref()?, prompt), deadline)?;
            return match out.trim() {
                "false" => Some(Choice::Deny),
                s => choice_from_label(&prompt.request, s),
            };
        }
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

/// Whether a desktop of this user is on screen.
/// - macOS: this user owns `/dev/console`. The login window or another user's desktop (fast
///   user switching) gives another owner.
/// - Linux: the X or Wayland display named in the env accepts a connection.
/// - Windows: always true. Its checks are in `Gui::available`.
#[cfg(unix)]
pub(super) fn desktop_on_screen(vars: &HashMap<String, String>) -> bool {
    use std::os::unix::fs::MetadataExt;
    if cfg!(target_os = "macos") {
        let uid = rustix::process::getuid().as_raw();
        return std::fs::metadata("/dev/console").is_ok_and(|m| m.uid() == uid);
    }
    display_sockets(vars).iter().any(Socket::accepts)
}

#[cfg(not(unix))]
pub(super) fn desktop_on_screen(_: &HashMap<String, String>) -> bool {
    true
}

#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(not(unix), allow(dead_code))]
enum Socket {
    Unix(PathBuf),
    Tcp(String, u16),
}

impl Socket {
    #[cfg(unix)]
    fn accepts(&self) -> bool {
        match self {
            Self::Unix(p) => std::os::unix::net::UnixStream::connect(p).is_ok(),
            Self::Tcp(host, port) => {
                use std::net::{TcpStream, ToSocketAddrs};
                (host.as_str(), *port).to_socket_addrs().is_ok_and(|mut a| {
                    a.any(|a| TcpStream::connect_timeout(&a, Duration::from_millis(300)).is_ok())
                })
            }
        }
    }
}

/// Sockets of the displays named by `DISPLAY` and `WAYLAND_DISPLAY`.
#[cfg_attr(not(unix), allow(dead_code))]
fn display_sockets(vars: &HashMap<String, String>) -> Vec<Socket> {
    let mut out = Vec::new();
    // `[host]:N[.screen]`. No host or `unix` means the local socket, else TCP port 6000+N.
    if let Some((host, rest)) = vars.get("DISPLAY").and_then(|d| d.rsplit_once(':'))
        && let Ok(n) = rest.split('.').next().unwrap_or("").parse::<u16>()
    {
        out.push(if host.is_empty() || host == "unix" {
            Socket::Unix(PathBuf::from(format!("/tmp/.X11-unix/X{n}")))
        } else {
            Socket::Tcp(host.to_string(), 6000u16.saturating_add(n))
        });
    }
    // A bare name lives in `XDG_RUNTIME_DIR`.
    if let Some(w) = vars.get("WAYLAND_DISPLAY") {
        let p = Path::new(w);
        if p.is_absolute() {
            out.push(Socket::Unix(p.to_path_buf()));
        } else if let Some(dir) = vars.get("XDG_RUNTIME_DIR") {
            out.push(Socket::Unix(Path::new(dir).join(p)));
        }
    }
    out
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

/// The Windows dialog. Fixed: everything it shows comes from `SAFESH_DIALOG_*` env vars. It calls
/// .NET directly and no cmdlets, so no module is autoloaded from `PSModulePath`. OK prints the
/// picked label. Cancel, Esc and the close box print `false`, which is Deny, as on macOS.
const WINDOWS_SCRIPT: &str = r"$ErrorActionPreference = 'Stop'
[void][Reflection.Assembly]::LoadWithPartialName('System.Windows.Forms')
[void][Reflection.Assembly]::LoadWithPartialName('System.Drawing')
[Windows.Forms.Application]::EnableVisualStyles()
$labels = $env:SAFESH_DIALOG_LABELS.Split([char]10)
$default = [int]$env:SAFESH_DIALOG_DEFAULT
$form = [Windows.Forms.Form]::new()
$form.Text = 'safesh'
$form.TopMost = $true
$form.StartPosition = 'CenterScreen'
$form.FormBorderStyle = 'FixedDialog'
$form.MinimizeBox = $false
$form.MaximizeBox = $false
$form.AutoSize = $true
$form.AutoSizeMode = 'GrowAndShrink'
$panel = [Windows.Forms.FlowLayoutPanel]::new()
$panel.FlowDirection = 'TopDown'
$panel.WrapContents = $false
$panel.AutoSize = $true
$panel.Padding = [Windows.Forms.Padding]::new(12)
$title = [Windows.Forms.Label]::new()
$title.Text = $env:SAFESH_DIALOG_TITLE
$title.AutoSize = $true
$title.Font = [Drawing.Font]::new($title.Font, [Drawing.FontStyle]::Bold)
$panel.Controls.Add($title)
$body = [Windows.Forms.TextBox]::new()
$body.Multiline = $true
$body.ReadOnly = $true
$body.WordWrap = $false
$body.ScrollBars = 'Both'
$body.Font = [Drawing.Font]::new('Consolas', 9)
$body.Width = 680
$body.Height = 170
$body.Text = $env:SAFESH_DIALOG_TEXT.Replace([string][char]10, [string][char]13 + [char]10)
$panel.Controls.Add($body)
$radios = [Collections.Generic.List[Windows.Forms.RadioButton]]::new()
for ($i = 0; $i -lt $labels.Length; $i++) {
    $r = [Windows.Forms.RadioButton]::new()
    $r.Text = $labels[$i]
    $r.AutoSize = $true
    $r.Checked = ($i -eq $default)
    $panel.Controls.Add($r)
    $radios.Add($r)
}
$buttons = [Windows.Forms.FlowLayoutPanel]::new()
$buttons.AutoSize = $true
$ok = [Windows.Forms.Button]::new()
$ok.Text = 'OK'
$ok.DialogResult = [Windows.Forms.DialogResult]::OK
$cancel = [Windows.Forms.Button]::new()
$cancel.Text = 'Cancel'
$cancel.DialogResult = [Windows.Forms.DialogResult]::Cancel
$buttons.Controls.Add($ok)
$buttons.Controls.Add($cancel)
$panel.Controls.Add($buttons)
$form.Controls.Add($panel)
$form.AcceptButton = $ok
$form.CancelButton = $cancel
$form.Add_Shown({ $form.Activate(); $ok.Focus() })
$result = $form.ShowDialog()
$picked = 'false'
if ($result -eq [Windows.Forms.DialogResult]::OK) {
    foreach ($r in $radios) { if ($r.Checked) { $picked = $r.Text } }
}
[Console]::Out.Write($picked)
";

/// `powershell.exe` with an empty env plus `SystemRoot`, `windir` and the dialog's data. The
/// agent's shell may have set `PSModulePath`, `COR_PROFILER` and the like, so nothing from the
/// calling env reaches the child.
fn windows_cmd(powershell: &Path, win_root: &Path, prompt: &Prompt) -> Command {
    let mut cmd = Command::new(powershell);
    cmd.env_clear()
        .env("SystemRoot", win_root)
        .env("windir", win_root)
        .env("SAFESH_DIALOG_TITLE", prompt.request.title())
        .env("SAFESH_DIALOG_TEXT", body_lines(prompt).join("\n"))
        .env(
            "SAFESH_DIALOG_LABELS",
            numbered_labels(&prompt.request).join("\n"),
        )
        .env("SAFESH_DIALOG_DEFAULT", deny_index(prompt).to_string())
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-EncodedCommand",
        ])
        .arg(encode_command(WINDOWS_SCRIPT));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        /// `CREATE_NO_WINDOW`: no console window flashes up behind the dialog.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// `-EncodedCommand` form: base64 of the UTF-16LE script.
fn encode_command(script: &str) -> String {
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    base64(&bytes)
}

fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ABC[(n >> (18 - 6 * i)) as usize & 63]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod windows_tests {
    use std::collections::HashMap;
    use std::path::Path;

    use super::super::Request;
    use super::*;

    fn prompt(c: &str) -> Prompt {
        Prompt {
            id: "p1".into(),
            cwd: r"C:\work".into(),
            request: Request::Command {
                commands: vec![c.into()],
                command: None,
            },
        }
    }

    fn envs(c: &Command) -> HashMap<String, String> {
        c.get_envs()
            .filter_map(|(k, v)| Some((k.to_str()?.to_string(), v?.to_str()?.to_string())))
            .collect()
    }

    #[test]
    fn base64_vectors() {
        for (i, o) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(i.as_bytes()), o);
        }
        assert_eq!(encode_command("a"), "YQA=");
    }

    #[test]
    fn hostile_text_reaches_the_dialog_only_as_env() {
        let evil = "'; Remove-Item -Recurse ~ ; $(calc) `\"x";
        let p = prompt(evil);
        let c = windows_cmd(Path::new(r"C:\W\ps.exe"), Path::new(r"C:\W"), &p);
        let args: Vec<String> = c
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args[..4],
            [
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-EncodedCommand"
            ]
        );
        assert_eq!(args.len(), 5);
        assert_eq!(args[4], encode_command(WINDOWS_SCRIPT));
        assert!(!WINDOWS_SCRIPT.contains("Remove-Item"));
        let e = envs(&c);
        assert!(e["SAFESH_DIALOG_TEXT"].contains("Remove-Item"));
        assert_eq!(e["SAFESH_DIALOG_TITLE"], p.request.title());
        assert_eq!(
            e["SAFESH_DIALOG_LABELS"],
            numbered_labels(&p.request).join("\n")
        );
        assert_eq!(e["SAFESH_DIALOG_DEFAULT"], deny_index(&p).to_string());
        assert_eq!(e["SystemRoot"], r"C:\W");
        let mut keys: Vec<&str> = e.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "SAFESH_DIALOG_DEFAULT",
                "SAFESH_DIALOG_LABELS",
                "SAFESH_DIALOG_TEXT",
                "SAFESH_DIALOG_TITLE",
                "SystemRoot",
                "windir"
            ]
        );
    }

    #[test]
    fn script_uses_no_cmdlets() {
        // A cmdlet would autoload its module from `PSModulePath`. Verb-Noun names are cmdlets.
        let verbs = [
            "Add-", "New-", "Write-", "Get-", "Set-", "Out-", "Where-", "ForEach-", "Select-",
            "Import-",
        ];
        for v in verbs {
            assert!(!WINDOWS_SCRIPT.contains(v), "{v}");
        }
    }

    #[test]
    fn available_on_windows_desktop_only() {
        let dir = tempfile::tempdir().unwrap();
        let ps = dir.path().join("powershell.exe");
        std::fs::write(&ps, "").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&ps, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let env = |vars: &[(&str, &str)], root: bool| Env {
            vars: vars
                .iter()
                .map(|(k, v)| ((*k).into(), (*v).into()))
                .collect(),
            windows: true,
            win_root: root.then(|| r"C:\Windows".into()),
            bin_dirs: vec![dir.path().to_path_buf()],
            ..Env::default()
        };
        assert!(Gui.available(&env(&[("SESSIONNAME", "Console")], true)));
        assert!(Gui.available(&env(&[("SESSIONNAME", "RDP-Tcp#3")], true)));
        assert!(!Gui.available(&env(&[], true)));
        assert!(!Gui.available(&env(&[("SESSIONNAME", "Console")], false)));
        for ssh in ["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"] {
            assert!(
                !Gui.available(&env(&[("SESSIONNAME", "Console"), (ssh, "x")], true)),
                "{ssh}"
            );
        }
        let mut no_ps = env(&[("SESSIONNAME", "Console")], true);
        no_ps.bin_dirs = vec![];
        assert!(!Gui.available(&no_ps));
    }
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
                command: None,
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
                desktop: true,
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
    fn no_desktop_on_screen_blocks_gui() {
        for (macos, vars, bin) in [
            (true, vec![], "osascript"),
            (false, vec![("DISPLAY", ":0")], "zenity"),
        ] {
            let (mut e, _d) = env(macos, &vars, &[bin]);
            assert!(Gui.available(&e));
            e.desktop = false;
            assert!(!Gui.available(&e));
        }
    }

    #[test]
    fn display_sockets_from_env() {
        let s = |vars: &[(&str, &str)]| {
            let vars: HashMap<String, String> = vars
                .iter()
                .map(|(k, v)| ((*k).into(), (*v).into()))
                .collect();
            display_sockets(&vars)
        };
        let x = |n: &str| Socket::Unix(PathBuf::from(format!("/tmp/.X11-unix/X{n}")));
        assert_eq!(s(&[("DISPLAY", ":0")]), [x("0")]);
        assert_eq!(s(&[("DISPLAY", ":1.0")]), [x("1")]);
        assert_eq!(s(&[("DISPLAY", "unix:2")]), [x("2")]);
        assert_eq!(
            s(&[("DISPLAY", "localhost:10.0")]),
            [Socket::Tcp("localhost".into(), 6010)]
        );
        assert_eq!(s(&[("DISPLAY", "garbage")]), []);
        assert_eq!(
            s(&[
                ("WAYLAND_DISPLAY", "wayland-0"),
                ("XDG_RUNTIME_DIR", "/run/user/1")
            ]),
            [Socket::Unix(PathBuf::from("/run/user/1/wayland-0"))]
        );
        assert_eq!(
            s(&[("WAYLAND_DISPLAY", "/w/sock")]),
            [Socket::Unix(PathBuf::from("/w/sock"))]
        );
        // A bare Wayland name without a runtime dir cannot be found.
        assert_eq!(s(&[("WAYLAND_DISPLAY", "wayland-0")]), []);
    }

    #[test]
    fn socket_accepts_only_a_listener() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("X0");
        assert!(!Socket::Unix(p.clone()).accepts());
        let _l = std::os::unix::net::UnixListener::bind(&p).unwrap();
        assert!(Socket::Unix(p).accepts());
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        assert!(Socket::Tcp("127.0.0.1".into(), port).accepts());
        drop(l);
        assert!(!Socket::Tcp("127.0.0.1".into(), port).accepts());
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
