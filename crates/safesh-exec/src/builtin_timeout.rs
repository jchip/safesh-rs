//! Built-in `timeout` command.
//!
//! Emulates GNU coreutils `timeout` so scripts running under safesh on macOS
//! (where `/usr/bin/timeout` is missing) can bound command execution.

use std::io::Write;
use std::time::Duration;

use brush_core::builtins::{ContentOptions, ContentType, Invocation, ParseArgs, UsageError};
use brush_core::commands::{ExecutionContext, ShellForCommand, SimpleCommand};
use brush_core::processes::ProcessWaitResult;
use brush_core::results::{ExecutionExitCode, ExecutionResult, ExecutionSpawnResult};
#[cfg(unix)]
use brush_core::traps::TrapSignal;
use brush_core::{CommandArg, Error};

#[cfg(unix)]
type TimeoutSignal = TrapSignal;

#[cfg(not(unix))]
#[derive(Debug, Clone, Copy)]
struct TimeoutSignal {
    name: &'static str,
    number: i32,
}

#[cfg(not(unix))]
impl std::fmt::Display for TimeoutSignal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name)
    }
}

#[derive(Debug)]
#[allow(clippy::struct_excessive_bools)]
pub struct TimeoutCommand {
    preserve_status: bool,
    #[allow(dead_code)]
    foreground: bool,
    verbose: bool,
    signal: TimeoutSignal,
    kill_after: Option<Duration>,
    duration: Duration,
    command_and_args: Vec<String>,
    help: bool,
    version: bool,
    error: Option<String>,
}

#[cfg(unix)]
fn default_signal() -> TimeoutSignal {
    "TERM"
        .parse::<TrapSignal>()
        .expect("TERM is a valid signal")
}

#[cfg(unix)]
fn kill_signal() -> TimeoutSignal {
    "KILL"
        .parse::<TrapSignal>()
        .expect("KILL is a valid signal")
}

#[cfg(unix)]
fn parse_signal(s: &str) -> Result<TimeoutSignal, String> {
    s.parse::<TrapSignal>()
        .map_err(|_| format!("invalid signal '{s}'"))
}

#[cfg(unix)]
fn signal_number(sig: TimeoutSignal) -> i32 {
    i32::try_from(sig).unwrap_or(15)
}

#[cfg(not(unix))]
const fn default_signal() -> TimeoutSignal {
    TimeoutSignal {
        name: "SIGTERM",
        number: 15,
    }
}

#[cfg(not(unix))]
const fn kill_signal() -> TimeoutSignal {
    TimeoutSignal {
        name: "SIGKILL",
        number: 9,
    }
}

#[cfg(not(unix))]
fn parse_signal(s: &str) -> Result<TimeoutSignal, String> {
    let upper = s.to_ascii_uppercase();
    let trimmed = upper.strip_prefix("SIG").unwrap_or(&upper);
    match trimmed {
        "HUP" | "1" => Ok(TimeoutSignal {
            name: "SIGHUP",
            number: 1,
        }),
        "INT" | "2" => Ok(TimeoutSignal {
            name: "SIGINT",
            number: 2,
        }),
        "QUIT" | "3" => Ok(TimeoutSignal {
            name: "SIGQUIT",
            number: 3,
        }),
        "ABRT" | "6" => Ok(TimeoutSignal {
            name: "SIGABRT",
            number: 6,
        }),
        "KILL" | "9" => Ok(kill_signal()),
        "ALRM" | "14" => Ok(TimeoutSignal {
            name: "SIGALRM",
            number: 14,
        }),
        "TERM" | "15" => Ok(default_signal()),
        _ => Err(format!("invalid signal '{s}'")),
    }
}

#[cfg(not(unix))]
const fn signal_number(sig: TimeoutSignal) -> i32 {
    sig.number
}

impl TimeoutCommand {
    fn err(msg: impl Into<String>) -> Self {
        Self {
            preserve_status: false,
            foreground: false,
            verbose: false,
            signal: default_signal(),
            kill_after: None,
            duration: Duration::ZERO,
            command_and_args: Vec::new(),
            help: false,
            version: false,
            error: Some(msg.into()),
        }
    }
}

fn parse_duration(s: &str) -> Result<Duration, String> {
    if s.is_empty() {
        return Err("empty duration".to_string());
    }
    let (num_str, multiplier) = if let Some(stripped) = s.strip_suffix('s') {
        (stripped, 1.0)
    } else if let Some(stripped) = s.strip_suffix('m') {
        (stripped, 60.0)
    } else if let Some(stripped) = s.strip_suffix('h') {
        (stripped, 3600.0)
    } else if let Some(stripped) = s.strip_suffix('d') {
        (stripped, 86400.0)
    } else {
        (s, 1.0)
    };
    let val: f64 = num_str
        .parse()
        .map_err(|_| format!("invalid time interval '{s}'"))?;
    if val < 0.0 || val.is_nan() || val.is_infinite() {
        return Err(format!("invalid time interval '{s}'"));
    }
    Ok(Duration::from_secs_f64(val * multiplier))
}

#[cfg(unix)]
#[allow(clippy::similar_names)]
fn kill_child(pid: i32, pgid: Option<i32>, sig: TimeoutSignal) {
    if let Some(pgid) = pgid {
        let shell_pgid = rustix::process::getpgrp().as_raw_nonzero().get();
        if pgid > 0 && pgid != shell_pgid {
            let _ = brush_core::sys::signal::kill_process(-pgid, sig);
        }
    }
    let _ = brush_core::sys::signal::kill_process(pid, sig);
}

#[cfg(windows)]
fn kill_child(pid: i32, _pgid: Option<i32>, _sig: TimeoutSignal) {
    if pid > 0 {
        safesh_policy::compat::terminate_and_wait(pid.cast_unsigned(), 124, 2000);
    }
}

#[cfg(not(any(unix, windows)))]
fn kill_child(_pid: i32, _pgid: Option<i32>, _sig: TrapSignal) {}

impl ParseArgs for TimeoutCommand {
    fn content(
        name: &str,
        content_type: ContentType,
        _options: &ContentOptions,
    ) -> Result<String, Error> {
        Ok(match content_type {
            ContentType::ShortUsage => {
                format!("{name}: {name} [OPTION] DURATION COMMAND [ARG]...\n")
            }
            _ => format!(
                "{name} - Run a command with a time limit.\n\n\
                 Start COMMAND, and kill it if still running after DURATION.\n"
            ),
        })
    }

    #[allow(clippy::too_many_lines)]
    fn parse(invocation: Invocation<'_>) -> Result<Self, UsageError> {
        let args: Vec<String> = invocation
            .into_args()
            .into_iter()
            .map(String::from)
            .collect();

        let mut preserve_status = false;
        let mut foreground = false;
        let mut verbose = false;
        let mut signal = default_signal();
        let mut kill_after = None;

        let mut iter = args.into_iter().peekable();
        let mut duration_str = None;

        while let Some(arg) = iter.next() {
            if arg == "--" {
                duration_str = iter.next();
                break;
            }
            if !arg.starts_with('-') || arg == "-" {
                duration_str = Some(arg);
                break;
            }

            match arg.as_str() {
                "--help" => {
                    return Ok(Self {
                        preserve_status,
                        foreground,
                        verbose,
                        signal,
                        kill_after,
                        duration: Duration::ZERO,
                        command_and_args: Vec::new(),
                        help: true,
                        version: false,
                        error: None,
                    });
                }
                "--version" => {
                    return Ok(Self {
                        preserve_status,
                        foreground,
                        verbose,
                        signal,
                        kill_after,
                        duration: Duration::ZERO,
                        command_and_args: Vec::new(),
                        help: false,
                        version: true,
                        error: None,
                    });
                }
                "--preserve-status" => preserve_status = true,
                "--foreground" => foreground = true,
                "-v" | "--verbose" => verbose = true,
                opt if opt.starts_with("--kill-after=") => {
                    let val = &opt["--kill-after=".len()..];
                    match parse_duration(val) {
                        Ok(d) => kill_after = Some(d),
                        Err(e) => return Ok(Self::err(e)),
                    }
                }
                "--kill-after" => {
                    let Some(val) = iter.next() else {
                        return Ok(Self::err("option '--kill-after' requires an argument"));
                    };
                    match parse_duration(&val) {
                        Ok(d) => kill_after = Some(d),
                        Err(e) => return Ok(Self::err(e)),
                    }
                }
                opt if opt.starts_with("-k") => {
                    let val = &opt[2..];
                    let val = if val.is_empty() {
                        let Some(v) = iter.next() else {
                            return Ok(Self::err("option requires an argument -- 'k'"));
                        };
                        v
                    } else {
                        val.to_string()
                    };
                    match parse_duration(&val) {
                        Ok(d) => kill_after = Some(d),
                        Err(e) => return Ok(Self::err(e)),
                    }
                }
                opt if opt.starts_with("--signal=") => {
                    let val = &opt["--signal=".len()..];
                    match parse_signal(val) {
                        Ok(s) => signal = s,
                        Err(e) => return Ok(Self::err(e)),
                    }
                }
                "--signal" => {
                    let Some(val) = iter.next() else {
                        return Ok(Self::err("option '--signal' requires an argument"));
                    };
                    match parse_signal(&val) {
                        Ok(s) => signal = s,
                        Err(e) => return Ok(Self::err(e)),
                    }
                }
                opt if opt.starts_with("-s") => {
                    let val = &opt[2..];
                    let val = if val.is_empty() {
                        let Some(v) = iter.next() else {
                            return Ok(Self::err("option requires an argument -- 's'"));
                        };
                        v
                    } else {
                        val.to_string()
                    };
                    match parse_signal(&val) {
                        Ok(s) => signal = s,
                        Err(e) => return Ok(Self::err(e)),
                    }
                }
                unknown => {
                    return Ok(Self::err(format!("unrecognized option '{unknown}'")));
                }
            }
        }

        let Some(d_str) = duration_str else {
            return Ok(Self::err("missing operand"));
        };

        let duration = match parse_duration(&d_str) {
            Ok(d) => d,
            Err(e) => return Ok(Self::err(e)),
        };

        let command_and_args: Vec<String> = iter.collect();
        if command_and_args.is_empty() {
            return Ok(Self::err("missing operand after duration"));
        }

        Ok(Self {
            preserve_status,
            foreground,
            verbose,
            signal,
            kill_after,
            duration,
            command_and_args,
            help: false,
            version: false,
            error: None,
        })
    }
}

impl<SE: brush_core::ShellExtensions> brush_core::builtins::Builtin<SE> for TimeoutCommand {
    type Error = Error;

    #[allow(
        clippy::too_many_lines,
        clippy::similar_names,
        clippy::single_match_else
    )]
    async fn execute(
        self,
        context: ExecutionContext<'_, SE>,
    ) -> Result<ExecutionResult, Self::Error> {
        if self.help {
            let _ = writeln!(
                context.stdout(),
                "Usage: timeout [OPTION] DURATION COMMAND [ARG]...\n\
                 Start COMMAND, and kill it if still running after DURATION."
            );
            return Ok(ExecutionResult::success());
        }

        if self.version {
            let _ = writeln!(context.stdout(), "safesh timeout (bundled coreutils) 0.1.0");
            return Ok(ExecutionResult::success());
        }

        if let Some(err) = self.error {
            let _ = writeln!(context.stderr(), "timeout: {err}");
            return Ok(ExecutionResult::from(ExecutionExitCode::from(125)));
        }

        let command_name = self.command_and_args[0].clone();
        let command_and_args = self.command_and_args.iter().map(CommandArg::from);

        let mut cmd = SimpleCommand::new(
            ShellForCommand::ParentShell(context.shell),
            context.params.clone(),
            command_name,
            command_and_args,
        );
        cmd.use_functions = true;

        let spawn_result = cmd.execute().await?;

        match spawn_result {
            ExecutionSpawnResult::Completed(res) => Ok(res),
            ExecutionSpawnResult::StartedProcess(mut child) => {
                let pid = child.pid();
                let pgid = child.pgid();
                if self.duration.is_zero() {
                    let wait_res = child.wait().await?;
                    return Ok(match wait_res {
                        ProcessWaitResult::Completed(res) => res,
                        ProcessWaitResult::Stopped => {
                            ExecutionResult::from(ExecutionExitCode::from(146))
                        }
                    });
                }

                match tokio::time::timeout(self.duration, child.wait()).await {
                    Ok(wait_result) => {
                        let r = wait_result?;
                        Ok(match r {
                            ProcessWaitResult::Completed(res) => res,
                            ProcessWaitResult::Stopped => {
                                ExecutionResult::from(ExecutionExitCode::from(146))
                            }
                        })
                    }
                    Err(_) => {
                        if self.verbose {
                            let _ = writeln!(
                                context.stderr(),
                                "timeout: sending signal {} to command '{}'",
                                self.signal,
                                self.command_and_args[0]
                            );
                        }
                        if let Some(pid) = pid {
                            kill_child(pid, pgid, self.signal);
                        }
                        let grace = self
                            .kill_after
                            .unwrap_or_else(|| Duration::from_millis(500));
                        if tokio::time::timeout(grace, child.wait()).await.is_err() {
                            if let Some(pid) = pid {
                                kill_child(pid, pgid, kill_signal());
                            }
                            let _ = tokio::time::timeout(Duration::from_millis(500), child.wait())
                                .await;
                        }

                        if self.preserve_status {
                            let sig_num = signal_number(self.signal);
                            #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                            Ok(ExecutionResult::from(ExecutionExitCode::from(
                                (128 + sig_num) as u8,
                            )))
                        } else {
                            Ok(ExecutionResult::from(ExecutionExitCode::from(124)))
                        }
                    }
                }
            }
            ExecutionSpawnResult::StartedTask(task) => {
                if self.duration.is_zero() {
                    let task_res = task.await??;
                    return Ok(task_res);
                }
                match tokio::time::timeout(self.duration, task).await {
                    Ok(task_res) => {
                        let res = task_res??;
                        Ok(res)
                    }
                    Err(_) => {
                        if self.preserve_status {
                            let sig_num = signal_number(self.signal);
                            #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                            Ok(ExecutionResult::from(ExecutionExitCode::from(
                                (128 + sig_num) as u8,
                            )))
                        } else {
                            Ok(ExecutionResult::from(ExecutionExitCode::from(124)))
                        }
                    }
                }
            }
        }
    }
}
