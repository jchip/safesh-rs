//! Bash command-line parsing for nested shells (`bash -c ...`, `sh script.sh`, `bash -lc ...`).

use std::path::PathBuf;

/// A parsed non-interactive bash invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellArgs {
    /// `$0`.
    pub shell_name: String,
    /// The `-c` string.
    pub command: Option<String>,
    /// Script file, when not `-c`. Neither means read the script from stdin.
    pub script: Option<PathBuf>,
    /// `$1`...
    pub positional: Vec<String>,
    /// Options to apply with `set` before running, such as `-e` or `-o pipefail`.
    pub set_options: Vec<String>,
}

/// Single-letter options that map to `set`.
const SET_FLAGS: &str = "abeEfhHkmnBCPtTuvx";

/// Parses bash arguments. `name` is `$0` when no `-c` name or script is given.
/// Interactive and login flags are accepted and ignored, since safesh never runs interactively.
pub fn parse_bash_args(name: &str, args: &[String]) -> Result<ShellArgs, String> {
    let mut out = ShellArgs {
        shell_name: name.to_string(),
        command: None,
        script: None,
        positional: Vec::new(),
        set_options: Vec::new(),
    };
    let mut command_mode = false;
    let mut stdin_mode = false;
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        i += 1;
        match a.as_str() {
            "--" => break,
            "--norc" | "--noprofile" | "--login" | "--posix" | "--noediting" | "--restricted" => {}
            "--rcfile" | "--init-file" => i += 1,
            _ if a.starts_with("--") => return Err(format!("unsupported option {a}")),
            _ if (a.starts_with('-') || a.starts_with('+')) && a.len() > 1 => {
                let sign = &a[..1];
                for flag in a[1..].chars() {
                    match flag {
                        'c' => command_mode = true,
                        's' => stdin_mode = true,
                        'i' | 'l' | 'r' => {}
                        'o' | 'O' => {
                            let Some(v) = args.get(i) else {
                                return Err(format!("{sign}{flag} needs an argument"));
                            };
                            i += 1;
                            if flag == 'o' {
                                out.set_options.push(format!("{sign}o"));
                                out.set_options.push(v.clone());
                            }
                        }
                        f if SET_FLAGS.contains(f) => out.set_options.push(format!("{sign}{f}")),
                        f => return Err(format!("unsupported option {sign}{f}")),
                    }
                }
            }
            _ => {
                i -= 1;
                break;
            }
        }
    }
    let mut rest = args[i..].iter().cloned();
    if command_mode {
        let cmd = rest.next().ok_or("-c needs a command string")?;
        out.command = Some(cmd);
        if let Some(n) = rest.next() {
            out.shell_name = n;
        }
    } else if !stdin_mode && let Some(script) = rest.next() {
        out.shell_name.clone_from(&script);
        out.script = Some(PathBuf::from(script));
    }
    out.positional = rest.collect();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(args: &[&str]) -> ShellArgs {
        let v: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
        parse_bash_args("bash", &v).unwrap()
    }

    #[test]
    fn dash_c_with_name_and_args() {
        let a = p(&["-c", "echo $0 $1", "me", "one"]);
        assert_eq!(a.command.as_deref(), Some("echo $0 $1"));
        assert_eq!(a.shell_name, "me");
        assert_eq!(a.positional, vec!["one"]);
    }

    #[test]
    fn login_cluster_and_set_flags() {
        let a = p(&["-lec", "true"]);
        assert_eq!(a.command.as_deref(), Some("true"));
        assert_eq!(a.set_options, vec!["-e"]);
        let a = p(&["--norc", "-o", "pipefail", "-x", "-c", "ls"]);
        assert_eq!(a.set_options, vec!["-o", "pipefail", "-x"]);
    }

    #[test]
    fn script_mode() {
        let a = p(&["-e", "s.sh", "a", "b"]);
        assert_eq!(a.script, Some(PathBuf::from("s.sh")));
        assert_eq!(a.shell_name, "s.sh");
        assert_eq!(a.positional, vec!["a", "b"]);
        assert_eq!(a.set_options, vec!["-e"]);
    }

    #[test]
    fn stdin_mode_and_errors() {
        let a = p(&["-s", "x"]);
        assert!(a.script.is_none() && a.command.is_none());
        assert_eq!(a.positional, vec!["x"]);
        assert!(parse_bash_args("bash", &["-c".into()]).is_err());
        assert!(parse_bash_args("bash", &["--version".into()]).is_err());
    }
}
