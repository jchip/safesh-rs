//! Spike: run bash through brush-core with a policy check at every exec and
//! every redirect/source open.
//!
//! Usage (bash-like): brush-policy [-e] [-u] [-x] [-o opt] -c 'script' [name args...]
//!                    brush-policy [opts] script.sh [args...]
//!
//! Env:
//!   SAFESH_SPIKE_DENY   comma list of command names to deny (default "rm")
//!   SAFESH_SPIKE_ROOTS  colon list of writable roots (default: cwd, /tmp, /dev)
//!   SAFESH_SPIKE_TRACE  "1" prints every allowed exec/open to stderr
//!   SAFESH_SPIKE_ABORT  "0" keeps running after a denial (default: stop with 126)

use brush_builtins::{BuiltinSet, ShellBuilderExt as _};
use brush_core::extensions::{DefaultProcessLauncher, ProcessLauncher, ShellExtensionsImpl};
use brush_core::processes::{ChildProcess, LaunchRequest};
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

const ROOTS_VAR: &str = "SAFESH_SPIKE_ROOTS";

/// Commands whose non-flag operands are write targets.
const PATH_WRITERS: &[&str] = &["rm", "rmdir", "mv", "cp", "touch", "mkdir", "chmod", "ln", "tee"];

#[derive(Debug, Default)]
struct Policy {
    deny: Vec<String>,
    roots: Vec<PathBuf>,
    roots_env: String,
    trace: bool,
    abort: bool,
    self_exe: PathBuf,
}

#[derive(Clone, Default)]
struct PolicyLauncher {
    policy: Arc<Policy>,
}

type SpikeExtensions = ShellExtensionsImpl<
    brush_core::extensions::DefaultErrorFormatter,
    PolicyLauncher,
    brush_core::extensions::DefaultTaskSpawner,
>;

impl PolicyLauncher {
    fn deny(&self, reason: String) -> io::Error {
        eprintln!("[SAFESH] BLOCKED: {reason}");
        if self.policy.abort {
            std::process::exit(126);
        }
        io::Error::new(io::ErrorKind::PermissionDenied, reason)
    }

    fn check_exec(&self, req: &LaunchRequest) -> Result<(), io::Error> {
        let p = &self.policy;
        let name = basename(&req.program);
        if p.trace {
            eprintln!(
                "[policy] exec {} argv0={} args={:?} cwd={}",
                req.program.display(),
                req.argv0,
                req.args,
                req.working_dir.display()
            );
        }
        if p.deny.iter().any(|d| d == &name) {
            return Err(self.deny(format!("command '{name}' is denied")));
        }
        if name == "git"
            && req.args.first().map(String::as_str) == Some("push")
            && req.args.iter().any(|a| a == "--force" || a == "-f")
        {
            return Err(self.deny("git push --force needs approval".into()));
        }
        if PATH_WRITERS.contains(&name.as_str()) {
            for arg in req.args.iter().filter(|a| !a.starts_with('-')) {
                let target = resolve(&req.working_dir.join(arg));
                if !self.writable(&target) {
                    return Err(self.deny(format!(
                        "{name} would write outside allowed roots: {}",
                        target.display()
                    )));
                }
            }
        }
        Ok(())
    }

    fn writable(&self, path: &Path) -> bool {
        self.policy.roots.iter().any(|r| path.starts_with(r))
    }

    /// Route nested `bash`/`sh` back into this binary so their commands are checked too.
    fn reroute_shell(&self, req: &mut LaunchRequest) {
        if matches!(basename(&req.program).as_str(), "bash" | "sh") {
            if self.policy.trace {
                eprintln!("[policy] nested shell -> {}", self.policy.self_exe.display());
            }
            req.program.clone_from(&self.policy.self_exe);
            req.env.retain(|(k, _)| k != ROOTS_VAR);
            req.env.push((ROOTS_VAR.into(), self.policy.roots_env.clone()));
        }
    }
}

impl ProcessLauncher for PolicyLauncher {
    fn spawn(&self, mut request: LaunchRequest) -> Result<ChildProcess, io::Error> {
        self.check_exec(&request)?;
        self.reroute_shell(&mut request);
        DefaultProcessLauncher.spawn(request)
    }

    fn exec(&self, mut request: LaunchRequest) -> io::Error {
        if let Err(e) = self.check_exec(&request) {
            return e;
        }
        self.reroute_shell(&mut request);
        DefaultProcessLauncher.exec(request)
    }

    fn check_open(&self, path: &Path, write: bool) -> Result<(), io::Error> {
        if self.policy.trace {
            eprintln!("[policy] open {} write={write}", path.display());
        }
        if write && !self.writable(&resolve(path)) {
            return Err(self.deny(format!("redirect writes outside allowed roots: {}", path.display())));
        }
        Ok(())
    }
}

fn basename(p: &Path) -> String {
    p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Absolute, lexically normalized path; symlinks resolved on the longest existing prefix.
fn resolve(path: &Path) -> PathBuf {
    let mut norm = PathBuf::new();
    for c in path.components() {
        match c {
            Component::ParentDir => {
                norm.pop();
            }
            Component::CurDir => {}
            other => norm.push(other),
        }
    }
    let mut existing = norm.clone();
    let mut rest = Vec::new();
    while !existing.exists() {
        match (existing.file_name().map(ToOwned::to_owned), existing.parent()) {
            (Some(name), Some(parent)) => {
                rest.push(name);
                existing = parent.to_path_buf();
            }
            _ => return norm,
        }
    }
    let mut out = existing.canonicalize().unwrap_or(existing);
    for name in rest.into_iter().rev() {
        out.push(name);
    }
    out
}

struct Args {
    set_opts: Vec<String>,
    command: Option<String>,
    script: Option<String>,
    rest: Vec<String>,
}

fn parse_args() -> Result<Args, String> {
    let mut it = std::env::args().skip(1).peekable();
    let mut a = Args { set_opts: vec![], command: None, script: None, rest: vec![] };
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-c" => {
                a.command = Some(it.next().ok_or("-c needs an argument")?);
                break;
            }
            "-o" => a.set_opts.push(format!("-o {}", it.next().ok_or("-o needs an argument")?)),
            "--" => break,
            s if s.starts_with('-') && s.len() > 1 && s[1..].chars().all(|c| "eux".contains(c)) => {
                a.set_opts.push(s.to_string());
            }
            s if s.starts_with('-') => return Err(format!("unsupported option {s}")),
            _ => {
                a.script = Some(arg);
                break;
            }
        }
    }
    a.rest = it.collect();
    if a.command.is_none() && a.script.is_none() {
        return Err("need -c 'script' or a script path".into());
    }
    Ok(a)
}

fn build_policy() -> Policy {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    let roots_env = std::env::var(ROOTS_VAR).unwrap_or_else(|_| format!("{}:/tmp:/dev", cwd.display()));
    let roots = roots_env.split(':').filter(|s| !s.is_empty()).map(|s| resolve(Path::new(s))).collect();
    let deny = std::env::var("SAFESH_SPIKE_DENY")
        .unwrap_or_else(|_| "rm".into())
        .split(',')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    Policy {
        deny,
        roots,
        roots_env,
        trace: std::env::var("SAFESH_SPIKE_TRACE").as_deref() == Ok("1"),
        abort: std::env::var("SAFESH_SPIKE_ABORT").as_deref() != Ok("0"),
        self_exe: std::env::current_exe().unwrap_or_else(|_| PathBuf::from("brush-policy")),
    }
}

#[tokio::main]
async fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("brush-policy: {e}");
            std::process::exit(2);
        }
    };
    let launcher = PolicyLauncher { policy: Arc::new(build_policy()) };

    let (name, shell_args) = match (&args.command, &args.script) {
        (Some(_), _) => {
            let mut rest = args.rest.clone().into_iter();
            (rest.next().unwrap_or_else(|| "bash".into()), rest.collect::<Vec<_>>())
        }
        (None, Some(script)) => (script.clone(), args.rest.clone()),
        _ => unreachable!(),
    };

    let mut shell = match brush_core::Shell::builder_with_extensions::<SpikeExtensions>()
        .process_launcher(launcher)
        .profile(brush_core::ProfileLoadBehavior::Skip)
        .rc(brush_core::RcLoadBehavior::Skip)
        .command_string_mode(args.command.is_some())
        .shell_name(name)
        .shell_args(shell_args.clone())
        .default_builtins(BuiltinSet::BashMode)
        .build()
        .await
    {
        Ok(s) => s,
        Err(e) => {
            eprintln!("brush-policy: {e}");
            std::process::exit(2);
        }
    };

    if !args.set_opts.is_empty() {
        let set_cmd = format!("set {}", args.set_opts.join(" "));
        let params = shell.default_exec_params();
        let _ = shell.run_string(set_cmd, &brush_core::SourceInfo::default(), &params).await;
    }

    let result = match (&args.command, &args.script) {
        (Some(cmd), _) => shell.run_dash_c_command(cmd.clone()).await,
        (None, Some(script)) => shell.run_script(script, shell_args.iter().cloned()).await,
        _ => unreachable!(),
    };
    let code = match result {
        Ok(r) => u8::from(r.exit_code),
        Err(e) => {
            eprintln!("brush-policy: {e}");
            1
        }
    };
    std::process::exit(i32::from(code));
}
