//! `safesh approve-ui`: the picker shown inside the tmux popup.
//!
//! It reads the prompt file, asks for a number, and writes the answer file. At the deadline,
//! or on end of input, it exits without an answer, which the caller treats as no answer.

use std::io::{BufRead, Write};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::{Prompt, body_lines, files, numbered_labels};

/// Popup width in columns, border included.
pub const POPUP_WIDTH: usize = 90;
const MAX_ROWS: usize = 40;

pub fn run(id: &str, state_dir: &Path, deadline_ms: u64) -> u8 {
    let Some(pf) = files::read_prompt(state_dir, id) else {
        eprintln!("safesh: no prompt {id}");
        return 1;
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let left = Duration::from_millis(deadline_ms).saturating_sub(now);
    std::thread::spawn(move || {
        std::thread::sleep(left);
        std::process::exit(0);
    });

    let req = &pf.prompt.request;
    let mut out = std::io::stdout();
    for l in screen(&pf.prompt) {
        let _ = writeln!(out, "{l}");
    }
    let n = req.choices().len();
    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();
    loop {
        let _ = write!(out, "{}", input_line(n));
        let _ = out.flush();
        let Some(Ok(line)) = lines.next() else {
            return 0;
        };
        if let Some(c) = parse(&line, n).map(|i| req.choices()[i]) {
            return u8::from(files::write_answer(state_dir, id, &pf.nonce, c).is_none());
        }
    }
}

/// Lines printed above the input line. The title is in the popup border, not here.
fn screen(prompt: &Prompt) -> Vec<String> {
    let mut out: Vec<String> = body_lines(prompt)
        .iter()
        .map(|l| format!("  {l}"))
        .collect();
    out.push(String::new());
    out.extend(numbered_labels(&prompt.request));
    out.push(String::new());
    out
}

fn input_line(n: usize) -> String {
    format!("choice (1-{n}): ")
}

/// Popup height: every printed row, with long lines wrapped, plus the border.
#[must_use]
pub fn popup_rows(prompt: &Prompt) -> usize {
    let mut lines = screen(prompt);
    lines.push(input_line(prompt.request.choices().len()));
    (lines.iter().map(|l| wrapped_rows(l)).sum::<usize>() + 2).min(MAX_ROWS)
}

fn wrapped_rows(line: &str) -> usize {
    line.chars().count().div_ceil(POPUP_WIDTH - 2).max(1)
}

fn parse(line: &str, n: usize) -> Option<usize> {
    let i: usize = line.trim().parse().ok()?;
    (1..=n).contains(&i).then(|| i - 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approval::Request;
    use safesh_policy::state::pending::PathOperation;

    fn path_prompt(command: &str) -> Prompt {
        Prompt {
            id: "p1".into(),
            cwd: "/Users/me/dev/proj".into(),
            request: Request::Path {
                path: "/Users/me/out.txt".into(),
                op: PathOperation::Write,
                command: Some(command.into()),
            },
        }
    }

    /// Rows of what `run` prints before input, counted from the text itself.
    fn printed_rows(p: &Prompt) -> usize {
        let mut text = String::new();
        for l in screen(p) {
            text.push_str(&l);
            text.push('\n');
        }
        text.push_str(&input_line(p.request.choices().len()));
        text.split('\n').map(wrapped_rows).sum()
    }

    #[test]
    fn popup_fits_everything_printed() {
        let long = "x".repeat(150);
        let many = (0..12)
            .map(|i| format!("echo {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        for p in [
            path_prompt("echo hi > /Users/me/out.txt"),
            path_prompt(&long),
            path_prompt(&many),
        ] {
            assert_eq!(popup_rows(&p), printed_rows(&p) + 2);
        }
    }

    #[test]
    fn path_screen_shows_path_and_command() {
        let s = screen(&path_prompt("echo hi > /Users/me/out.txt"));
        assert_eq!(s[0], "  path: /Users/me/out.txt");
        assert_eq!(s[1], "  command: echo hi > /Users/me/out.txt");
        assert_eq!(s[3], "  in /Users/me/dev/proj");
    }

    #[test]
    fn command_screen_shows_full_command() {
        let p = Prompt {
            id: "p1".into(),
            cwd: "/Users/me/dev/proj".into(),
            request: Request::Command {
                commands: vec!["node".into()],
                command: Some("node scripts/build.js --prod".into()),
            },
        };
        let s = screen(&p);
        assert_eq!(s[0], "  node");
        assert_eq!(s[1], "  command: node scripts/build.js --prod");
        assert_eq!(s[3], "  in /Users/me/dev/proj");
    }

    #[test]
    fn parse_range() {
        assert_eq!(parse(" 1\n", 5), Some(0));
        assert_eq!(parse("5", 5), Some(4));
        assert_eq!(parse("0", 5), None);
        assert_eq!(parse("5", 4), None);
        assert_eq!(parse("y", 5), None);
    }
}
