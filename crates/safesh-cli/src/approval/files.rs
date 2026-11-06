//! Files under `<state>/approvals-ui/`: the prompt queue, and prompt and answer files.
//!
//! The dir is under the state dir, which is on the deny-write list, so the agent's shell
//! can't write answers or tickets.

use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use safesh_policy::state::temp::check_id;
use serde::{Deserialize, Serialize};

use super::{Choice, Prompt};

const POLL: Duration = Duration::from_millis(100);

pub fn ui_dir(state_dir: &Path) -> PathBuf {
    state_dir.join("approvals-ui")
}

fn ensure_dir(dir: &Path) -> std::io::Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
}

/// This process's place in the prompt queue. Dropping it leaves the queue.
pub struct Queue {
    ticket: PathBuf,
    _lock: File,
}

impl Queue {
    /// Joins the queue and waits until this prompt is first in line. Tickets sort by arrival
    /// time, so prompts show in order. Tickets whose process died are removed. Returns `None`
    /// if `give_up_at` passes first.
    pub fn wait_turn(state_dir: &Path, id: &str, give_up_at: Instant) -> Option<Self> {
        check_id(id).ok()?;
        let dir = ui_dir(state_dir).join("queue");
        ensure_dir(&dir).ok()?;
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()?
            .as_nanos();
        let pid = std::process::id();
        let ticket = dir.join(format!("{nanos:024}-{pid}-{id}"));
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&ticket)
            .ok()?;
        let lock_path = ui_dir(state_dir).join("lock");
        loop {
            if first_live_ticket(&dir).as_deref() == Some(ticket.as_path())
                && let Some(lock) = try_lock(&lock_path)
            {
                return Some(Self {
                    ticket,
                    _lock: lock,
                });
            }
            if Instant::now() >= give_up_at {
                let _ = fs::remove_file(&ticket);
                return None;
            }
            sleep(POLL);
        }
    }
}

impl Drop for Queue {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.ticket);
    }
}

/// The OS lock backs up the ticket order: it is freed on a crash, and two processes can't
/// both show a prompt if ticket names ever tie.
fn try_lock(path: &Path) -> Option<File> {
    let f = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)
        .ok()?;
    f.try_lock().ok()?;
    Some(f)
}

fn first_live_ticket(dir: &Path) -> Option<PathBuf> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .collect();
    names.sort();
    for name in names {
        let path = dir.join(&name);
        if ticket_alive(&name) {
            return Some(path);
        }
        let _ = fs::remove_file(path);
    }
    None
}

fn ticket_alive(name: &str) -> bool {
    let Some(pid) = name.split('-').nth(1).and_then(|p| p.parse::<i32>().ok()) else {
        return false;
    };
    let Some(pid) = rustix::process::Pid::from_raw(pid) else {
        return false;
    };
    // EPERM means the process exists but belongs to someone else.
    !matches!(
        rustix::process::test_kill_process(pid),
        Err(rustix::io::Errno::SRCH)
    )
}

#[derive(Serialize, Deserialize)]
pub struct PromptFile {
    pub prompt: Prompt,
    pub nonce: String,
}

#[derive(Serialize, Deserialize)]
struct AnswerFile {
    nonce: String,
    choice: Choice,
}

fn prompt_path(state_dir: &Path, id: &str) -> PathBuf {
    ui_dir(state_dir).join(format!("{id}.prompt.json"))
}

fn answer_path(state_dir: &Path, id: &str) -> PathBuf {
    ui_dir(state_dir).join(format!("{id}.answer.json"))
}

/// Writes the prompt file for `approve-ui` and returns the nonce its answer must carry.
pub fn write_prompt(state_dir: &Path, prompt: &Prompt) -> Option<String> {
    check_id(&prompt.id).ok()?;
    ensure_dir(&ui_dir(state_dir)).ok()?;
    clear(state_dir, &prompt.id);
    let nonce = nonce()?;
    let body = serde_json::to_vec(&PromptFile {
        prompt: prompt.clone(),
        nonce: nonce.clone(),
    })
    .ok()?;
    write_new(&prompt_path(state_dir, &prompt.id), &body)?;
    Some(nonce)
}

pub fn read_prompt(state_dir: &Path, id: &str) -> Option<PromptFile> {
    check_id(id).ok()?;
    serde_json::from_slice(&fs::read(prompt_path(state_dir, id)).ok()?).ok()
}

/// Called by `approve-ui` with the nonce from the prompt file.
pub fn write_answer(state_dir: &Path, id: &str, nonce: &str, choice: Choice) -> Option<()> {
    check_id(id).ok()?;
    let body = serde_json::to_vec(&AnswerFile {
        nonce: nonce.to_string(),
        choice,
    })
    .ok()?;
    let tmp = ui_dir(state_dir).join(format!(".{id}.answer.tmp"));
    let _ = fs::remove_file(&tmp);
    write_new(&tmp, &body)?;
    fs::rename(&tmp, answer_path(state_dir, id)).ok()
}

/// Reads the answer, accepts it only with the right nonce, then removes both files.
pub fn take_answer(state_dir: &Path, id: &str, nonce: &str) -> Option<Choice> {
    let a: Option<AnswerFile> = fs::read(answer_path(state_dir, id))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok());
    clear(state_dir, id);
    a.filter(|a| a.nonce == nonce).map(|a| a.choice)
}

pub fn clear(state_dir: &Path, id: &str) {
    let _ = fs::remove_file(prompt_path(state_dir, id));
    let _ = fs::remove_file(answer_path(state_dir, id));
}

fn write_new(path: &Path, body: &[u8]) -> Option<()> {
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .ok()?;
    f.write_all(body).ok()
}

fn nonce() -> Option<String> {
    let mut b = [0u8; 16];
    File::open("/dev/urandom").ok()?.read_exact(&mut b).ok()?;
    Some(b.iter().fold(String::new(), |mut s, x| {
        let _ = write!(s, "{x:02x}");
        s
    }))
}
