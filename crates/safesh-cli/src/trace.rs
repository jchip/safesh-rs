//! Event log and live status, for finding out why a command seems to do nothing.
//!
//! Each hook, run, retry and nested shell adds JSON lines to `<state>/log/<date>.jsonl`.
//! While one is in progress, `<state>/active/<pid>.json` says what it is doing right now.
//! `safesh status` reads the active files and `safesh log` reads the log. With
//! `SAFESH_DEBUG=1`, each event is also printed to stderr.
//!
//! Tracing never fails a command: every write error is ignored.

use std::fmt::Write as _;
use std::io::{BufRead as _, Seek as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};

/// Log files older than this many days are removed when a new day's file starts.
const KEEP_DAYS: i64 = 7;
/// Longest command or argv text kept in one event.
const PREVIEW: usize = 200;

static TRACER: OnceLock<Tracer> = OnceLock::new();

struct Tracer {
    state_dir: PathBuf,
    kind: &'static str,
    started: Instant,
    started_at: String,
    /// `SAFESH_DEBUG=1`: also print each event to stderr.
    echo: bool,
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    id: Option<String>,
    session: Option<String>,
    cwd: Option<String>,
    /// What the process is doing now, and since when.
    step: String,
    step_at: String,
    ended: bool,
}

fn now_local() -> String {
    chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, false)
}

/// Cuts `s` to at most `PREVIEW` chars.
pub fn preview(s: &str) -> String {
    if s.chars().count() <= PREVIEW {
        return s.to_string();
    }
    let mut out: String = s.chars().take(PREVIEW).collect();
    out.push('…');
    out
}

fn log_dir(state_dir: &Path) -> PathBuf {
    state_dir.join("log")
}

fn active_dir(state_dir: &Path) -> PathBuf {
    state_dir.join("active")
}

fn private_dir(dir: &Path) -> std::io::Result<()> {
    if !dir.is_dir() {
        std::fs::create_dir_all(dir)?;
        safesh_policy::compat::restrict_dir(dir)?;
    }
    Ok(())
}

/// Starts tracing this process. Later calls are ignored. `kind` is `hook`, `run`, `retry` or
/// `shell`, and the first event is `<kind>.start` with `fields` and the process cwd.
pub fn start(
    state_dir: &Path,
    kind: &'static str,
    id: Option<&str>,
    session: Option<&str>,
    fields: Value,
) {
    let cwd = std::env::current_dir()
        .ok()
        .map(|c| c.to_string_lossy().into_owned());
    let t = Tracer {
        state_dir: state_dir.to_path_buf(),
        echo: std::env::var("SAFESH_DEBUG").is_ok_and(|v| v == "1"),
        kind,
        started: Instant::now(),
        started_at: now_local(),
        inner: Mutex::new(Inner {
            id: id.map(str::to_string),
            session: session.map(str::to_string),
            cwd: cwd.clone(),
            step: "starting".into(),
            step_at: now_local(),
            ended: false,
        }),
    };
    if TRACER.set(t).is_err() {
        return;
    }
    let mut fields = fields;
    if let Value::Object(m) = &mut fields {
        m.insert("cwd".into(), cwd.into());
    }
    event(&format!("{kind}.start"), fields);
    write_active();
}

/// Adds the script id, session and cwd to later events, when known.
pub fn set_context(id: Option<&str>, session: Option<&str>, cwd: Option<&Path>) {
    let Some(t) = TRACER.get() else { return };
    {
        let Ok(mut g) = t.inner.lock() else { return };
        if let Some(i) = id {
            g.id = Some(i.to_string());
        }
        if let Some(s) = session {
            g.session = Some(s.to_string());
        }
        if let Some(c) = cwd {
            g.cwd = Some(c.to_string_lossy().into_owned());
        }
    }
    write_active();
}

/// Appends one event. `fields` must be a JSON object; anything else is dropped.
pub fn event(ev: &str, fields: Value) {
    let Some(t) = TRACER.get() else { return };
    let mut line = Map::new();
    line.insert("ts".into(), now_local().into());
    line.insert("pid".into(), std::process::id().into());
    line.insert("kind".into(), t.kind.into());
    line.insert("ev".into(), ev.into());
    if let Ok(g) = t.inner.lock() {
        if let Some(i) = &g.id {
            line.insert("id".into(), i.clone().into());
        }
        if let Some(s) = &g.session {
            line.insert("session".into(), s.clone().into());
        }
    }
    if let Value::Object(f) = fields {
        line.extend(f.into_iter().filter(|(_, v)| !v.is_null()));
    }
    let line = Value::Object(line);
    if t.echo {
        eprintln!("[safesh] {}", format_line(&line));
    }
    let _ = append(&t.state_dir, &line);
}

fn append(state_dir: &Path, line: &Value) -> std::io::Result<()> {
    let dir = log_dir(state_dir);
    private_dir(&dir)?;
    let path = dir.join(format!("{}.jsonl", chrono::Local::now().format("%Y-%m-%d")));
    if !path.exists() {
        prune(&dir);
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    let mut f = opts.open(path)?;
    // One write per line, so lines from concurrent processes don't interleave.
    let mut text = line.to_string();
    text.push('\n');
    f.write_all(text.as_bytes())
}

/// Removes log files older than `KEEP_DAYS`.
fn prune(dir: &Path) {
    let cutoff = (chrono::Local::now() - chrono::Duration::days(KEEP_DAYS))
        .format("%Y-%m-%d")
        .to_string();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if let Some(date) = name.strip_suffix(".jsonl")
            && date.len() == 10
            && date < cutoff.as_str()
        {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// Records what the process is doing now, for `safesh status`.
pub fn step(text: impl Into<String>) {
    let Some(t) = TRACER.get() else { return };
    {
        let Ok(mut g) = t.inner.lock() else { return };
        g.step = text.into();
        g.step_at = now_local();
    }
    write_active();
}

fn active_path(state_dir: &Path) -> PathBuf {
    active_dir(state_dir).join(format!("{}.json", std::process::id()))
}

fn write_active() {
    let Some(t) = TRACER.get() else { return };
    let Ok(g) = t.inner.lock() else { return };
    if g.ended {
        return;
    }
    let v = json!({
        "pid": std::process::id(),
        "kind": t.kind,
        "id": g.id,
        "session": g.session,
        "cwd": g.cwd,
        "started": t.started_at,
        "step": g.step,
        "step_at": g.step_at,
    });
    drop(g);
    let dir = active_dir(&t.state_dir);
    if private_dir(&dir).is_err() {
        return;
    }
    // Write then rename, so a reader never sees half a file.
    let path = active_path(&t.state_dir);
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, v.to_string()).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// Logs `<kind>.end` with the exit code and removes the active file.
pub fn end(code: u8) {
    let Some(t) = TRACER.get() else { return };
    {
        let Ok(mut g) = t.inner.lock() else { return };
        if g.ended {
            return;
        }
        g.ended = true;
    }
    let ms = u64::try_from(t.started.elapsed().as_millis()).unwrap_or(u64::MAX);
    event(
        &format!("{}.end", t.kind),
        json!({ "code": code, "ms": ms }),
    );
    let _ = std::fs::remove_file(active_path(&t.state_dir));
}

// ---------- safesh status ----------

fn age(since: &str) -> String {
    let Ok(t) = chrono::DateTime::parse_from_rfc3339(since) else {
        return "?".into();
    };
    let secs = (chrono::Local::now().fixed_offset() - t)
        .num_seconds()
        .max(0);
    match secs {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m{:02}s", s / 60, s % 60),
        s => format!("{}h{:02}m", s / 3600, (s % 3600) / 60),
    }
}

fn str_of<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}

/// `safesh status`: lists hooks and runs in progress. Entries whose process is gone are shown
/// once, then removed.
pub fn status(state_dir: &Path) -> u8 {
    let dir = active_dir(state_dir);
    let mut rows: Vec<(Value, bool)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_none_or(|x| x != "json") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&p) else {
                continue;
            };
            let Ok(v) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            let alive = v["pid"]
                .as_i64()
                .and_then(|p| i32::try_from(p).ok())
                .is_some_and(safesh_policy::compat::pid_alive);
            if !alive {
                let _ = std::fs::remove_file(&p);
            }
            rows.push((v, alive));
        }
    }
    if rows.is_empty() {
        println!("nothing in progress");
        return 0;
    }
    rows.sort_by(|a, b| str_of(&a.0, "started").cmp(str_of(&b.0, "started")));
    println!(
        "{:<7} {:<6} {:<8} {:<17} {:<10} NOW",
        "PID", "KIND", "AGE", "ID", "SESSION"
    );
    for (v, alive) in &rows {
        let session: String = str_of(v, "session").chars().take(8).collect();
        let now = if *alive {
            format!("{} ({})", str_of(v, "step"), age(str_of(v, "step_at")))
        } else {
            format!("gone without finishing; last step: {}", str_of(v, "step"))
        };
        println!(
            "{:<7} {:<6} {:<8} {:<17} {:<10} {now}",
            v["pid"].to_string(),
            str_of(v, "kind"),
            age(str_of(v, "started")),
            str_of(v, "id"),
            session,
        );
    }
    0
}

// ---------- safesh log ----------

pub struct LogQuery {
    /// Script or pending id, or a prefix of one.
    pub id: Option<String>,
    pub session: Option<String>,
    /// `YYYY-MM-DD`; today by default.
    pub date: Option<String>,
    pub lines: usize,
    pub follow: bool,
}

fn matches(v: &Value, q: &LogQuery) -> bool {
    let id_ok = q.id.as_deref().is_none_or(|want| {
        ["id", "pending"]
            .iter()
            .any(|k| str_of(v, k).starts_with(want))
    });
    let session_ok = q
        .session
        .as_deref()
        .is_none_or(|want| str_of(v, "session").starts_with(want));
    id_ok && session_ok
}

/// One event as a line of text: time, pid, kind, event, id, then the other fields.
fn format_line(v: &Value) -> String {
    let ts = str_of(v, "ts");
    let time = ts.get(11..23).unwrap_or(ts);
    let mut out = format!(
        "{time} {:<6} {:<6} {:<16} {:<16}",
        v["pid"].to_string(),
        str_of(v, "kind"),
        str_of(v, "ev"),
        str_of(v, "id"),
    );
    if let Value::Object(m) = v {
        for (k, val) in m {
            if matches!(k.as_str(), "ts" | "pid" | "kind" | "ev" | "id" | "session") {
                continue;
            }
            let text = match val {
                Value::String(s) => s.replace('\n', "\\n"),
                other => other.to_string(),
            };
            let _ = write!(out, " {k}={text}");
        }
    }
    out
}

/// `safesh log`: prints the last matching events, and with `follow`, new ones as they come.
pub fn log(state_dir: &Path, q: &LogQuery) -> u8 {
    let date = q
        .date
        .clone()
        .unwrap_or_else(|| chrono::Local::now().format("%Y-%m-%d").to_string());
    let path = log_dir(state_dir).join(format!("{date}.jsonl"));
    let mut pos = 0;
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            pos = text.len() as u64;
            let hits: Vec<Value> = text
                .lines()
                .filter_map(|l| serde_json::from_str::<Value>(l).ok())
                .filter(|v| matches(v, q))
                .collect();
            for v in &hits[hits.len().saturating_sub(q.lines)..] {
                println!("{}", format_line(v));
            }
        }
        Err(_) if q.follow => {}
        Err(_) => {
            println!("no log for {date} at {}", path.display());
            return 0;
        }
    }
    if q.follow {
        follow(&path, pos, q);
    }
    0
}

/// Polls `path` for new lines until killed. Only reads today's file, as given.
fn follow(path: &Path, mut pos: u64, q: &LogQuery) {
    loop {
        if let Ok(mut f) = std::fs::File::open(path)
            && f.seek(std::io::SeekFrom::Start(pos)).is_ok()
        {
            let mut r = std::io::BufReader::new(&mut f);
            let mut line = String::new();
            // Only whole lines; a partial last line is read on the next poll.
            while r.read_line(&mut line).is_ok_and(|n| n > 0) && line.ends_with('\n') {
                pos += line.len() as u64;
                if let Ok(v) = serde_json::from_str::<Value>(&line)
                    && matches(&v, q)
                {
                    println!("{}", format_line(&v));
                    let _ = std::io::stdout().flush();
                }
                line.clear();
            }
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_cuts_long_text() {
        assert_eq!(preview("abc"), "abc");
        let long = "x".repeat(PREVIEW + 5);
        assert_eq!(preview(&long).chars().count(), PREVIEW + 1);
    }

    #[test]
    fn match_by_id_prefix_or_pending() {
        let q = LogQuery {
            id: Some("ab12".into()),
            session: None,
            date: None,
            lines: 10,
            follow: false,
        };
        assert!(matches(&json!({"id": "ab12cd"}), &q));
        assert!(matches(&json!({"id": "zz", "pending": "ab12ff"}), &q));
        assert!(!matches(&json!({"id": "zz"}), &q));
    }

    #[test]
    fn line_format_puts_extra_fields_last() {
        let v = json!({
            "ts": "2026-11-12T10:11:12.345-08:00", "pid": 7, "kind": "run",
            "ev": "exec", "id": "abc", "argv": "ls -l", "decision": "allow"
        });
        let s = format_line(&v);
        assert!(s.starts_with("10:11:12.345 7"), "{s}");
        assert!(s.contains("argv=ls -l"), "{s}");
        assert!(s.contains("decision=allow"), "{s}");
    }

    #[test]
    fn prune_removes_only_old_dated_files() {
        let td = tempfile::tempdir().unwrap();
        let old = td.path().join("2000-01-01.jsonl");
        let today = td
            .path()
            .join(format!("{}.jsonl", chrono::Local::now().format("%Y-%m-%d")));
        let other = td.path().join("notes.txt");
        for p in [&old, &today, &other] {
            std::fs::write(p, "").unwrap();
        }
        prune(td.path());
        assert!(!old.exists());
        assert!(today.exists());
        assert!(other.exists());
    }
}
