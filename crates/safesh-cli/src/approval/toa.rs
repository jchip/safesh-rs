//! TOA (Team of Agents) approval channel.
//!
//! Submits interactive approval requests to TOA Gateway Server via HTTP/1.1 loopback
//! (`POST /v1/approvals`) and maps decisions back to `SafeShell` approval choices.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use safesh_policy::state::pending::PathOperation;
use serde::{Deserialize, Serialize};

use super::{Channel, Choice, Env, Prompt, Request};

#[cfg(test)]
thread_local! {
    static TEST_OVERRIDE_ENV: std::cell::RefCell<Option<Env>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
struct ResetGuard;

#[cfg(test)]
impl Drop for ResetGuard {
    fn drop(&mut self) {
        TEST_OVERRIDE_ENV.with(|cell| {
            *cell.borrow_mut() = None;
        });
    }
}

#[cfg(test)]
pub fn with_test_env<R>(env: Env, f: impl FnOnce() -> R) -> R {
    TEST_OVERRIDE_ENV.with(|cell| {
        *cell.borrow_mut() = Some(env);
    });
    let _guard = ResetGuard;
    f()
}

/// Dynamic approval choice sent to the client/dashboard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalChoice {
    pub id: String,
    pub label: String,
    pub hotkey: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_access_mode: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_modes: Option<Vec<String>>,
}

/// Request for approving a command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandApprovalRequest {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cli: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub cwd: String,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_name: Option<String>,
    pub timeout_secs: u64,
}

/// Request for approving access to a path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PathApprovalRequest {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cli: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub cwd: String,
    pub target_path: String,
    pub access_needed: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_name: Option<String>,
    pub timeout_secs: u64,
}

/// Outcome of a command approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalOutcome {
    AllowOnce,
    AllowSession,
    AllowProject,
    AllowGlobal,
    Deny,
}

impl ApprovalOutcome {
    #[allow(dead_code)]
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AllowOnce => "allow_once",
            Self::AllowSession => "allow_session",
            Self::AllowProject => "allow_project",
            Self::AllowGlobal => "allow_global",
            Self::Deny => "deny",
        }
    }
}

impl From<ApprovalOutcome> for Choice {
    fn from(outcome: ApprovalOutcome) -> Self {
        match outcome {
            ApprovalOutcome::AllowOnce => Choice::Once,
            ApprovalOutcome::AllowSession => Choice::Session,
            ApprovalOutcome::AllowProject => Choice::Project,
            ApprovalOutcome::AllowGlobal => Choice::Global,
            ApprovalOutcome::Deny => Choice::Deny,
        }
    }
}

/// Access mode granted for a path approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathAccessMode {
    Read,
    Write,
    ReadWrite,
}

impl PathAccessMode {
    #[allow(dead_code)]
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::ReadWrite => "read_write",
        }
    }
}

/// Outcome of a path approval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathApprovalOutcome {
    AllowOnce { access: PathAccessMode },
    AllowSession { access: PathAccessMode },
    AllowProject { access: PathAccessMode },
    Deny,
}

impl PathApprovalOutcome {
    #[allow(dead_code)]
    #[must_use]
    pub fn access(&self) -> Option<PathAccessMode> {
        match self {
            Self::AllowOnce { access }
            | Self::AllowSession { access }
            | Self::AllowProject { access } => Some(*access),
            Self::Deny => None,
        }
    }
}

impl From<PathApprovalOutcome> for Choice {
    fn from(outcome: PathApprovalOutcome) -> Self {
        match outcome {
            PathApprovalOutcome::AllowOnce { .. } => Choice::Once,
            PathApprovalOutcome::AllowSession { .. } => Choice::Session,
            PathApprovalOutcome::AllowProject { .. } => Choice::Project,
            PathApprovalOutcome::Deny => Choice::Deny,
        }
    }
}

/// Choices offered for command approvals (1..5).
pub fn command_choices() -> Vec<ApprovalChoice> {
    vec![
        ApprovalChoice {
            id: "allow_once".into(),
            label: "Allow once".into(),
            hotkey: "1".into(),
            requires_access_mode: None,
            access_modes: None,
        },
        ApprovalChoice {
            id: "allow_project".into(),
            label: "Always allow (this project)".into(),
            hotkey: "2".into(),
            requires_access_mode: None,
            access_modes: None,
        },
        ApprovalChoice {
            id: "allow_session".into(),
            label: "Allow for session".into(),
            hotkey: "3".into(),
            requires_access_mode: None,
            access_modes: None,
        },
        ApprovalChoice {
            id: "deny".into(),
            label: "Deny".into(),
            hotkey: "4".into(),
            requires_access_mode: None,
            access_modes: None,
        },
        ApprovalChoice {
            id: "allow_global".into(),
            label: "Always allow (all projects)".into(),
            hotkey: "5".into(),
            requires_access_mode: None,
            access_modes: None,
        },
    ]
}

/// Choices offered for path approvals (1..4 with access modes on allow).
pub fn path_choices() -> Vec<ApprovalChoice> {
    vec![
        ApprovalChoice {
            id: "allow_once".into(),
            label: "Allow once".into(),
            hotkey: "1".into(),
            requires_access_mode: Some(true),
            access_modes: Some(vec!["read".into(), "write".into(), "read_write".into()]),
        },
        ApprovalChoice {
            id: "allow_session".into(),
            label: "Allow for session".into(),
            hotkey: "2".into(),
            requires_access_mode: Some(true),
            access_modes: Some(vec!["read".into(), "write".into(), "read_write".into()]),
        },
        ApprovalChoice {
            id: "allow_project".into(),
            label: "Always allow (this project)".into(),
            hotkey: "3".into(),
            requires_access_mode: Some(true),
            access_modes: Some(vec!["read".into(), "write".into(), "read_write".into()]),
        },
        ApprovalChoice {
            id: "deny".into(),
            label: "Deny".into(),
            hotkey: "4".into(),
            requires_access_mode: Some(false),
            access_modes: None,
        },
    ]
}

/// Discovered endpoint and auth token for TOA Gateway.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToaDiscovery {
    pub gateway_url: String,
    pub hook_token: String,
    pub terminal_id: Option<String>,
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn parse_runtime_file(path: &Path) -> Option<(Option<String>, Option<String>)> {
    let content = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&content).ok()?;

    let gateway_url = v
        .get("gatewayUrl")
        .or_else(|| v.get("url"))
        .and_then(|s| s.as_str())
        .map(String::from)
        .or_else(|| {
            v.get("port")
                .and_then(serde_json::Value::as_u64)
                .map(|p| format!("http://127.0.0.1:{p}"))
        });

    let hook_token = v
        .get("hookToken")
        .or_else(|| v.get("token"))
        .or_else(|| v.get("bearerToken"))
        .and_then(|s| s.as_str())
        .map(String::from);

    Some((gateway_url, hook_token))
}

/// Discovers TOA endpoint, token, and optional terminal ID from env or config files.
pub fn discover_toa(env: Option<&Env>) -> Option<ToaDiscovery> {
    #[cfg(test)]
    let override_env = TEST_OVERRIDE_ENV.with(|cell| cell.borrow().clone());
    #[cfg(test)]
    let env = env.or(override_env.as_ref());

    let mut gateway_url = env
        .and_then(|e| e.var("TOA_GATEWAY_URL"))
        .map(String::from)
        .or_else(|| {
            std::env::var("TOA_GATEWAY_URL")
                .ok()
                .filter(|s| !s.is_empty())
        });

    let mut hook_token = env
        .and_then(|e| e.var("TOA_HOOK_TOKEN"))
        .map(String::from)
        .or_else(|| {
            std::env::var("TOA_HOOK_TOKEN")
                .ok()
                .filter(|s| !s.is_empty())
        });

    // If either is missing, check ~/.toa/runtime.json
    if gateway_url.is_none() || hook_token.is_none() {
        let home = env
            .and_then(|e| e.var("HOME").or_else(|| e.var("USERPROFILE")))
            .map(PathBuf::from)
            .or_else(home_dir);
        if let Some(home) = home {
            let runtime_file = home.join(".toa").join("runtime.json");
            if let Some((url, token)) = parse_runtime_file(&runtime_file) {
                if gateway_url.is_none() {
                    gateway_url = url;
                }
                if hook_token.is_none() {
                    hook_token = token;
                }
            }
        }
    }

    // Fallback: check TOA_API_DISCOVERY path if still missing
    if gateway_url.is_none() || hook_token.is_none() {
        let disco_path = env
            .and_then(|e| e.var("TOA_API_DISCOVERY"))
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("TOA_API_DISCOVERY").map(PathBuf::from));
        if let Some((url, token)) = disco_path.as_deref().and_then(parse_runtime_file) {
            if gateway_url.is_none() {
                gateway_url = url;
            }
            if hook_token.is_none() {
                hook_token = token;
            }
        }
    }

    let gateway_url = gateway_url?;
    let hook_token = hook_token?;

    let terminal_id = env
        .and_then(|e| e.var("TOA_TERMINAL_ID"))
        .map(String::from)
        .or_else(|| {
            std::env::var("TOA_TERMINAL_ID")
                .ok()
                .filter(|s| !s.is_empty())
        });

    Some(ToaDiscovery {
        gateway_url: gateway_url.trim_end_matches('/').to_string(),
        hook_token,
        terminal_id,
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ToaApprovalPayload<'a> {
    request_id: &'a str,
    kind: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    cli: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    terminal_id: Option<&'a str>,
    cwd: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    command: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_path: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    access_needed: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rule_name: Option<&'a str>,
    timeout_secs: u64,
    choices: Vec<ApprovalChoice>,
}

/// Zero-dependency HTTP/1.1 client over `TcpStream`.
fn http_post(url: &str, token: &str, body: &str, timeout_secs: u64) -> Option<(u16, String)> {
    let rest = url.strip_prefix("http://")?;
    let (host_port, path) = match rest.find('/') {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        None => (rest, "/"),
    };
    let path = if path.is_empty() { "/" } else { path };

    let (host, port) = match host_port.rfind(':') {
        Some(idx) => (&host_port[..idx], host_port[idx + 1..].parse::<u16>().ok()?),
        None => (host_port, 80),
    };

    let timeout = Duration::from_secs(timeout_secs.max(1));
    let addrs: Vec<SocketAddr> = (host, port).to_socket_addrs().ok()?.collect();

    let mut stream = None;
    let connect_timeout = timeout.min(Duration::from_secs(5));
    for addr in addrs {
        if let Ok(s) = TcpStream::connect_timeout(&addr, connect_timeout) {
            stream = Some(s);
            break;
        }
    }
    let mut stream = stream?;

    stream.set_read_timeout(Some(timeout)).ok()?;
    stream.set_write_timeout(Some(timeout)).ok()?;

    let req = format!(
        "POST {path} HTTP/1.1\r\n\
         Host: {host_port}\r\n\
         Authorization: Bearer {token}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {body}",
        body.len()
    );

    stream.write_all(req.as_bytes()).ok()?;
    stream.flush().ok()?;

    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let mut content_length: Option<usize> = None;
    let mut header_len: Option<usize> = None;

    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if header_len.is_none() {
                    let pos = buf.windows(4).position(|w| w == b"\r\n\r\n");
                    if let Some(pos) = pos {
                        let hlen = pos + 4;
                        header_len = Some(hlen);
                        let headers_str = String::from_utf8_lossy(&buf[..pos]);
                        for line in headers_str.lines() {
                            let Some((k, v)) = line.split_once(':') else {
                                continue;
                            };
                            if k.trim().eq_ignore_ascii_case("content-length") {
                                content_length = v.trim().parse::<usize>().ok();
                            }
                        }
                    }
                }
                if let (Some(hlen), Some(clen)) = (header_len, content_length) {
                    let total = hlen + clen;
                    if buf.len() >= total {
                        buf.truncate(total);
                        break;
                    }
                }
            }
            Err(_) => return None,
        }
    }

    let hlen = header_len?;
    let headers_str = std::str::from_utf8(&buf[..hlen]).ok()?;
    let status_line = headers_str.lines().next()?;
    let status_code = status_line.split_whitespace().nth(1)?.parse::<u16>().ok()?;
    let body_str = std::str::from_utf8(&buf[hlen..]).ok()?.to_string();

    Some((status_code, body_str))
}

/// Sends a command approval request to TOA Gateway.
pub fn send_command_request(
    disco: &ToaDiscovery,
    req: &CommandApprovalRequest,
) -> Option<ApprovalOutcome> {
    let payload = ToaApprovalPayload {
        request_id: &req.id,
        kind: "command",
        cli: req.cli.as_deref(),
        session_id: req.session_id.as_deref(),
        terminal_id: disco.terminal_id.as_deref(),
        cwd: &req.cwd,
        command: Some(&req.command),
        target_path: None,
        access_needed: None,
        reason: req.reason.as_deref(),
        rule_name: req.rule_name.as_deref(),
        timeout_secs: req.timeout_secs,
        choices: command_choices(),
    };

    let body = serde_json::to_string(&payload).ok()?;
    let endpoint = format!("{}/v1/approvals", disco.gateway_url.trim_end_matches('/'));
    let (status, resp_body) = http_post(&endpoint, &disco.hook_token, &body, req.timeout_secs)?;
    if status != 200 {
        return None;
    }

    let val: serde_json::Value = serde_json::from_str(&resp_body).ok()?;
    if val.get("fallback").and_then(serde_json::Value::as_bool) == Some(true) {
        return None;
    }

    let decision = val.get("decision").and_then(|d| d.as_str())?;
    match decision {
        "allow_once" | "1" => Some(ApprovalOutcome::AllowOnce),
        "allow_session" | "3" => Some(ApprovalOutcome::AllowSession),
        "allow_project" | "2" => Some(ApprovalOutcome::AllowProject),
        "allow_global" | "5" => Some(ApprovalOutcome::AllowGlobal),
        "deny" | "4" => Some(ApprovalOutcome::Deny),
        _ => None,
    }
}

/// Sends a path approval request to TOA Gateway.
pub fn send_path_request(
    disco: &ToaDiscovery,
    req: &PathApprovalRequest,
) -> Option<PathApprovalOutcome> {
    let payload = ToaApprovalPayload {
        request_id: &req.id,
        kind: "path",
        cli: req.cli.as_deref(),
        session_id: req.session_id.as_deref(),
        terminal_id: disco.terminal_id.as_deref(),
        cwd: &req.cwd,
        command: None,
        target_path: Some(&req.target_path),
        access_needed: Some(&req.access_needed),
        reason: req.reason.as_deref(),
        rule_name: req.rule_name.as_deref(),
        timeout_secs: req.timeout_secs,
        choices: path_choices(),
    };

    let body = serde_json::to_string(&payload).ok()?;
    let endpoint = format!("{}/v1/approvals", disco.gateway_url.trim_end_matches('/'));
    let (status, resp_body) = http_post(&endpoint, &disco.hook_token, &body, req.timeout_secs)?;
    if status != 200 {
        return None;
    }

    let val: serde_json::Value = serde_json::from_str(&resp_body).ok()?;
    if val.get("fallback").and_then(serde_json::Value::as_bool) == Some(true) {
        return None;
    }

    let decision = val.get("decision").and_then(|d| d.as_str())?;
    let access_str = val
        .get("access")
        .or_else(|| val.get("accessMode"))
        .and_then(|a| a.as_str());

    let (norm_decision, decision_access) = parse_path_decision(decision);
    let explicit_access = access_str.and_then(parse_path_access_str);
    let default_access = parse_path_access_str(&req.access_needed).unwrap_or(PathAccessMode::ReadWrite);

    let access = explicit_access
        .or(decision_access)
        .unwrap_or(default_access);

    match norm_decision {
        "allow_once" | "1" => Some(PathApprovalOutcome::AllowOnce { access }),
        "allow_session" | "2" => Some(PathApprovalOutcome::AllowSession { access }),
        "allow_project" | "3" => Some(PathApprovalOutcome::AllowProject { access }),
        "deny" | "4" => Some(PathApprovalOutcome::Deny),
        _ => None,
    }
}

fn parse_path_access_str(s: &str) -> Option<PathAccessMode> {
    match s.trim().to_lowercase().as_str() {
        "read" | "r" | "ro" => Some(PathAccessMode::Read),
        "write" | "w" => Some(PathAccessMode::Write),
        "read_write" | "rw" => Some(PathAccessMode::ReadWrite),
        _ => None,
    }
}

fn parse_path_decision(decision: &str) -> (&str, Option<PathAccessMode>) {
    let lower = decision.trim().to_lowercase();
    if let Some(rest) = lower.strip_prefix("rw") {
        if matches!(rest, "1" | "2" | "3") {
            return (scope_num_to_decision(rest), Some(PathAccessMode::ReadWrite));
        }
    } else if let Some(rest) = lower.strip_prefix("ro") {
        if matches!(rest, "1" | "2" | "3") {
            return (scope_num_to_decision(rest), Some(PathAccessMode::Read));
        }
    } else if let Some(rest) = lower.strip_prefix('r') {
        if matches!(rest, "1" | "2" | "3") {
            return (scope_num_to_decision(rest), Some(PathAccessMode::Read));
        }
    } else if let Some(rest) = lower.strip_prefix('w') {
        if matches!(rest, "1" | "2" | "3") {
            return (scope_num_to_decision(rest), Some(PathAccessMode::Write));
        }
    }
    (decision, None)
}

fn scope_num_to_decision(num: &str) -> &'static str {
    match num {
        "1" => "allow_once",
        "2" => "allow_session",
        "3" => "allow_project",
        _ => "allow_once",
    }
}

/// Resolves a command approval with an optional environment.
#[allow(dead_code)]
pub fn resolve_approval_with_env(
    channels: &[impl AsRef<str>],
    env: Option<&Env>,
    req: &CommandApprovalRequest,
) -> Option<ApprovalOutcome> {
    for ch in channels {
        if ch.as_ref() == "toa"
            && let Some(outcome) = discover_toa(env).and_then(|d| send_command_request(&d, req))
        {
            return Some(outcome);
        }
    }
    None
}

/// Resolves a command approval through the configured channel order.
#[allow(dead_code)]
pub fn resolve_approval(
    channels: &[impl AsRef<str>],
    req: &CommandApprovalRequest,
) -> Option<ApprovalOutcome> {
    resolve_approval_with_env(channels, None, req)
}

/// Resolves a path approval with an optional environment.
#[allow(dead_code)]
pub fn resolve_path_approval_with_env(
    channels: &[impl AsRef<str>],
    env: Option<&Env>,
    req: &PathApprovalRequest,
) -> Option<PathApprovalOutcome> {
    for ch in channels {
        if ch.as_ref() == "toa"
            && let Some(outcome) = discover_toa(env).and_then(|d| send_path_request(&d, req))
        {
            return Some(outcome);
        }
    }
    None
}

/// Resolves a path approval through the configured channel order.
#[allow(dead_code)]
pub fn resolve_path_approval(
    channels: &[impl AsRef<str>],
    req: &PathApprovalRequest,
) -> Option<PathApprovalOutcome> {
    resolve_path_approval_with_env(channels, None, req)
}

/// TOA approval channel implementing [`Channel`].
#[derive(Debug, Clone, Default)]
pub struct ToaChannel;

#[allow(dead_code)]
pub type Toa = ToaChannel;

impl Channel for ToaChannel {
    fn available(&self, env: &Env) -> bool {
        discover_toa(Some(env)).is_some()
    }

    fn ask(&self, env: &Env, prompt: &Prompt, deadline: Instant) -> Option<Choice> {
        let disco = discover_toa(Some(env))?;
        let timeout_secs = deadline
            .saturating_duration_since(Instant::now())
            .as_secs()
            .max(1);

        match &prompt.request {
            Request::Command { commands } => {
                let req = CommandApprovalRequest {
                    id: prompt.id.clone(),
                    cli: None,
                    session_id: None,
                    cwd: prompt.cwd.clone(),
                    command: commands.join(" && "),
                    reason: None,
                    rule_name: None,
                    timeout_secs,
                };
                let outcome = send_command_request(&disco, &req)?;
                Some(outcome.into())
            }
            Request::Path { path, op, command } => {
                let access = match op {
                    PathOperation::Read => "read",
                    PathOperation::Write => "write",
                };
                let req = PathApprovalRequest {
                    id: prompt.id.clone(),
                    cli: None,
                    session_id: None,
                    cwd: prompt.cwd.clone(),
                    target_path: path.clone(),
                    access_needed: access.to_string(),
                    reason: command.clone(),
                    rule_name: None,
                    timeout_secs,
                };
                let outcome = send_path_request(&disco, &req)?;
                Some(outcome.into())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    struct MockServer {
        pub addr: SocketAddr,
        pub requests: Arc<Mutex<Vec<(String, serde_json::Value)>>>,
        listener_shutdown: Arc<std::sync::atomic::AtomicBool>,
        handle: Option<std::thread::JoinHandle<()>>,
    }

    impl MockServer {
        fn start(status_code: u16, response_body: &'static str) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let requests = Arc::new(Mutex::new(Vec::new()));
            let shutdown = Arc::new(std::sync::atomic::AtomicBool::new(false));

            let reqs_clone = Arc::clone(&requests);
            let shutdown_clone = Arc::clone(&shutdown);

            listener.set_nonblocking(true).expect("set nonblocking");

            let handle = std::thread::spawn(move || {
                while !shutdown_clone.load(std::sync::atomic::Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            let mut buf = Vec::new();
                            let mut chunk = [0u8; 1024];
                            let mut content_length = None;
                            let mut header_len = None;

                            loop {
                                match stream.read(&mut chunk) {
                                    Ok(0) => break,
                                    Ok(n) => {
                                        buf.extend_from_slice(&chunk[..n]);
                                        if header_len.is_none() {
                                            let pos = buf.windows(4).position(|w| w == b"\r\n\r\n");
                                            if let Some(pos) = pos {
                                                header_len = Some(pos + 4);
                                                let h_str = String::from_utf8_lossy(&buf[..pos]);
                                                for line in h_str.lines() {
                                                    let Some((k, v)) = line.split_once(':') else {
                                                        continue;
                                                    };
                                                    if k.trim()
                                                        .eq_ignore_ascii_case("content-length")
                                                    {
                                                        content_length =
                                                            v.trim().parse::<usize>().ok();
                                                    }
                                                }
                                            }
                                        }
                                        if let (Some(hl), Some(cl)) = (header_len, content_length) {
                                            let total = hl + cl;
                                            if buf.len() >= total {
                                                break;
                                            }
                                        }
                                    }
                                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                                        std::thread::sleep(Duration::from_millis(5));
                                    }
                                    Err(_) => break,
                                }
                            }

                            if let Some(hl) = header_len {
                                let body_slice = &buf[hl..];
                                let json_val: serde_json::Value =
                                    serde_json::from_slice(body_slice).unwrap_or_default();
                                let req_str = String::from_utf8_lossy(&buf).to_string();
                                reqs_clone.lock().unwrap().push((req_str, json_val));
                            }

                            let resp = format!(
                                "HTTP/1.1 {status_code} OK\r\n\
                                 Content-Type: application/json\r\n\
                                 Content-Length: {}\r\n\
                                 Connection: close\r\n\
                                 \r\n\
                                 {response_body}",
                                response_body.len()
                            );
                            let _ = stream.write_all(resp.as_bytes());
                            let _ = stream.flush();
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        Err(_) => break,
                    }
                }
            });

            Self {
                addr,
                requests,
                listener_shutdown: shutdown,
                handle: Some(handle),
            }
        }
    }

    impl Drop for MockServer {
        fn drop(&mut self) {
            self.listener_shutdown
                .store(true, std::sync::atomic::Ordering::Relaxed);
            if let Some(h) = self.handle.take() {
                let _ = h.join();
            }
        }
    }

    #[test]
    fn test_command_approval_5_decisions() {
        let cases = [
            ("allow_once", ApprovalOutcome::AllowOnce),
            ("allow_session", ApprovalOutcome::AllowSession),
            ("allow_project", ApprovalOutcome::AllowProject),
            ("allow_global", ApprovalOutcome::AllowGlobal),
            ("deny", ApprovalOutcome::Deny),
        ];

        for (decision_str, expected) in cases {
            let body = Box::leak(format!(r#"{{"decision":"{decision_str}"}}"#).into_boxed_str());
            let server = MockServer::start(200, body);

            let mut env = Env::default();
            env.vars
                .insert("TOA_GATEWAY_URL".into(), format!("http://{}", server.addr));
            env.vars
                .insert("TOA_HOOK_TOKEN".into(), "test-token".into());

            let req = CommandApprovalRequest {
                id: "cmd-req-1".into(),
                cli: Some("claude".into()),
                session_id: Some("sess-1".into()),
                cwd: "/repo".into(),
                command: "npm test".into(),
                reason: Some("run tests".into()),
                rule_name: Some("npm-test-rule".into()),
                timeout_secs: 5,
            };

            let outcome = with_test_env(env, || resolve_approval(&["toa", "ask"], &req));
            assert_eq!(outcome, Some(expected));
        }
    }

    #[test]
    fn test_path_approval_access_modes() {
        let cases = [
            (
                r#"{"decision":"allow_once","access":"read"}"#,
                PathApprovalOutcome::AllowOnce {
                    access: PathAccessMode::Read,
                },
            ),
            (
                r#"{"decision":"allow_once","accessMode":"ro"}"#,
                PathApprovalOutcome::AllowOnce {
                    access: PathAccessMode::Read,
                },
            ),
            (
                r#"{"decision":"allow_session","access":"write"}"#,
                PathApprovalOutcome::AllowSession {
                    access: PathAccessMode::Write,
                },
            ),
            (
                r#"{"decision":"allow_project","access":"read_write"}"#,
                PathApprovalOutcome::AllowProject {
                    access: PathAccessMode::ReadWrite,
                },
            ),
            (
                r#"{"decision":"rw3","accessMode":"rw"}"#,
                PathApprovalOutcome::AllowProject {
                    access: PathAccessMode::ReadWrite,
                },
            ),
            (
                r#"{"decision":"w2"}"#,
                PathApprovalOutcome::AllowSession {
                    access: PathAccessMode::Write,
                },
            ),
            (r#"{"decision":"deny"}"#, PathApprovalOutcome::Deny),
        ];

        for (body, expected) in cases {
            let server = MockServer::start(200, body);

            let mut env = Env::default();
            env.vars
                .insert("TOA_GATEWAY_URL".into(), format!("http://{}", server.addr));
            env.vars
                .insert("TOA_HOOK_TOKEN".into(), "test-token".into());

            let req = PathApprovalRequest {
                id: "path-req-1".into(),
                cli: Some("codex".into()),
                session_id: Some("sess-2".into()),
                cwd: "/repo".into(),
                target_path: "/repo/secret.txt".into(),
                access_needed: "read_write".into(),
                reason: Some("read file".into()),
                rule_name: Some("path-rule".into()),
                timeout_secs: 5,
            };

            let outcome = with_test_env(env, || resolve_path_approval(&["toa", "ask"], &req));
            assert_eq!(outcome, Some(expected));
        }
    }

    #[test]
    fn test_terminal_id_and_dynamic_choices() {
        let server = MockServer::start(200, r#"{"decision":"allow_once"}"#);

        let mut env = Env::default();
        env.vars
            .insert("TOA_GATEWAY_URL".into(), format!("http://{}", server.addr));
        env.vars
            .insert("TOA_HOOK_TOKEN".into(), "test-token".into());
        env.vars.insert("TOA_TERMINAL_ID".into(), "term-42".into());

        let cmd_req = CommandApprovalRequest {
            id: "cmd-dyn".into(),
            cli: Some("agy".into()),
            session_id: Some("sess-42".into()),
            cwd: "/app".into(),
            command: "git push".into(),
            reason: Some("push branch".into()),
            rule_name: Some("git-push".into()),
            timeout_secs: 5,
        };

        let outcome = with_test_env(env.clone(), || resolve_approval(&["toa"], &cmd_req));
        assert_eq!(outcome, Some(ApprovalOutcome::AllowOnce));

        // Check posted JSON payload
        {
            let reqs = server.requests.lock().unwrap();
            assert_eq!(reqs.len(), 1);
            let (raw_req, json_body) = &reqs[0];
            assert!(raw_req.contains("Authorization: Bearer test-token"));
            assert_eq!(json_body["requestId"], "cmd-dyn");
            assert_eq!(json_body["kind"], "command");
            assert_eq!(json_body["cli"], "agy");
            assert_eq!(json_body["sessionId"], "sess-42");
            assert_eq!(json_body["terminalId"], "term-42");
            assert_eq!(json_body["cwd"], "/app");
            assert_eq!(json_body["command"], "git push");
            assert_eq!(json_body["reason"], "push branch");
            assert_eq!(json_body["ruleName"], "git-push");

            let choices = json_body["choices"].as_array().expect("choices array");
            assert_eq!(choices.len(), 5);
            assert_eq!(choices[0]["id"], "allow_once");
            assert_eq!(choices[0]["hotkey"], "1");
            assert_eq!(choices[1]["id"], "allow_project");
            assert_eq!(choices[1]["hotkey"], "2");
            assert_eq!(choices[2]["id"], "allow_session");
            assert_eq!(choices[2]["hotkey"], "3");
            assert_eq!(choices[3]["id"], "deny");
            assert_eq!(choices[3]["hotkey"], "4");
            assert_eq!(choices[4]["id"], "allow_global");
            assert_eq!(choices[4]["hotkey"], "5");
        }

        // Now test path approval dynamic choices (4 choices)
        let path_req = PathApprovalRequest {
            id: "path-dyn".into(),
            cli: Some("agy".into()),
            session_id: Some("sess-42".into()),
            cwd: "/app".into(),
            target_path: "/app/data.db".into(),
            access_needed: "write".into(),
            reason: Some("write database".into()),
            rule_name: None,
            timeout_secs: 5,
        };

        let path_outcome = with_test_env(env, || resolve_path_approval(&["toa"], &path_req));
        assert!(path_outcome.is_some());

        {
            let reqs = server.requests.lock().unwrap();
            assert_eq!(reqs.len(), 2);
            let (_, json_body) = &reqs[1];
            assert_eq!(json_body["requestId"], "path-dyn");
            assert_eq!(json_body["kind"], "path");
            assert_eq!(json_body["terminalId"], "term-42");
            assert_eq!(json_body["targetPath"], "/app/data.db");
            assert_eq!(json_body["accessNeeded"], "write");

            let choices = json_body["choices"].as_array().expect("choices array");
            assert_eq!(choices.len(), 4);
            assert_eq!(choices[0]["id"], "allow_once");
            assert_eq!(choices[0]["hotkey"], "1");
            assert_eq!(choices[0]["requiresAccessMode"], true);
            assert_eq!(choices[1]["id"], "allow_session");
            assert_eq!(choices[1]["hotkey"], "2");
            assert_eq!(choices[1]["requiresAccessMode"], true);
            assert_eq!(choices[2]["id"], "allow_project");
            assert_eq!(choices[2]["hotkey"], "3");
            assert_eq!(choices[2]["requiresAccessMode"], true);
            assert_eq!(choices[3]["id"], "deny");
            assert_eq!(choices[3]["hotkey"], "4");
            assert_eq!(choices[3]["requiresAccessMode"], false);
        }
    }

    #[test]
    fn test_fallback_unreachable_and_503() {
        let req = CommandApprovalRequest {
            id: "cmd-fb".into(),
            cli: None,
            session_id: None,
            cwd: "/cwd".into(),
            command: "echo test".into(),
            reason: None,
            rule_name: None,
            timeout_secs: 1,
        };

        // 1. Unreachable port
        let mut env = Env::default();
        env.vars
            .insert("TOA_GATEWAY_URL".into(), "http://127.0.0.1:59999".into());
        env.vars.insert("TOA_HOOK_TOKEN".into(), "tok".into());
        let outcome = with_test_env(env, || resolve_approval(&["toa", "ask"], &req));
        assert_eq!(outcome, None);

        // 2. HTTP 503 Service Unavailable
        let server_503 = MockServer::start(503, r#"{"error":"service unavailable"}"#);
        let mut env = Env::default();
        env.vars.insert(
            "TOA_GATEWAY_URL".into(),
            format!("http://{}", server_503.addr),
        );
        env.vars.insert("TOA_HOOK_TOKEN".into(), "tok".into());
        let outcome = with_test_env(env, || resolve_approval(&["toa", "ask"], &req));
        assert_eq!(outcome, None);

        // 3. HTTP 200 with fallback: true
        let server_fb = MockServer::start(200, r#"{"fallback":true}"#);
        let mut env = Env::default();
        env.vars.insert(
            "TOA_GATEWAY_URL".into(),
            format!("http://{}", server_fb.addr),
        );
        env.vars.insert("TOA_HOOK_TOKEN".into(), "tok".into());
        let outcome = with_test_env(env, || resolve_approval(&["toa", "ask"], &req));
        assert_eq!(outcome, None);
    }

    #[test]
    fn test_runtime_json_discovery() {
        let td = tempfile::tempdir().unwrap();
        let runtime_file = td.path().join("runtime.json");
        std::fs::write(
            &runtime_file,
            r#"{"gatewayUrl":"http://127.0.0.1:45123","hookToken":"tok-from-file"}"#,
        )
        .unwrap();

        let mut env = Env::default();
        env.vars.insert(
            "TOA_API_DISCOVERY".into(),
            runtime_file.to_string_lossy().into(),
        );

        let disco = with_test_env(env, || discover_toa(None));
        assert!(disco.is_some());
        let disco = disco.unwrap();
        assert_eq!(disco.gateway_url, "http://127.0.0.1:45123");
        assert_eq!(disco.hook_token, "tok-from-file");

        let disco_none = discover_toa(None);
        assert!(disco_none.is_none());
    }

    #[test]
    fn test_channel_trait_ask_integration() {
        let server = MockServer::start(200, r#"{"decision":"allow_project"}"#);

        let channel = ToaChannel;
        let mut env = Env::default();
        env.vars
            .insert("TOA_GATEWAY_URL".into(), format!("http://{}", server.addr));
        env.vars.insert("TOA_HOOK_TOKEN".into(), "trait-tok".into());

        assert!(channel.available(&env));

        let prompt = Prompt {
            id: "prompt-1".into(),
            cwd: "/home/user".into(),
            request: Request::Command {
                commands: vec!["cargo build".into()],
            },
        };

        let choice = channel.ask(&env, &prompt, Instant::now() + Duration::from_secs(5));
        assert_eq!(choice, Some(Choice::Project));
    }
}
