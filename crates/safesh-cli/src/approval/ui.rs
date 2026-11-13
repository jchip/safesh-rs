//! `safesh approve-ui`: the picker shown inside the tmux popup.
//!
//! It reads the prompt file, lets the user pick a choice, and writes the answer file. At the
//! deadline, on end of input, or on Ctrl-C, it exits without an answer, which the caller treats
//! as no answer.
//!
//! The cursor starts on a "Pick a selection" row, so a stray Enter typed into the agent pane
//! does nothing. Arrows, j/k or a digit move the cursor, and only Enter picks.

use std::io::{Read, Write};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::{Prompt, Request, body_lines, files, numbered_labels};

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
    #[cfg(unix)]
    let _raw = raw::Raw::enter();
    let mut out = std::io::stdout();
    for l in screen(&pf.prompt) {
        let _ = write!(out, "{l}\r\n");
    }
    let mut picker = Picker::new(req.choices().len());
    draw(&mut out, req, picker.sel, false);
    let mut bytes = std::io::stdin().lock().bytes().map_while(Result::ok);
    while let Some(k) = next_key(&mut bytes) {
        match picker.key(k) {
            Step::Pick(i) => {
                let c = req.choices()[i];
                return u8::from(files::write_answer(state_dir, id, &pf.nonce, c).is_none());
            }
            Step::Cancel => return 0,
            Step::Redraw => draw(&mut out, req, picker.sel, true),
            Step::Ignore => {}
        }
    }
    0
}

/// Lines printed above the choices. The title is in the popup border, not here.
fn screen(prompt: &Prompt) -> Vec<String> {
    let mut out: Vec<String> = body_lines(prompt)
        .iter()
        .map(|l| format!("  {l}"))
        .collect();
    out.push(String::new());
    out
}

/// The placeholder row, then one row per choice. Row `sel` is marked.
fn choice_rows(req: &Request, sel: usize) -> Vec<String> {
    let n = req.choices().len();
    let mut rows = vec![format!("Pick a selection (up/down or 1-{n}, then Enter)")];
    rows.extend(numbered_labels(req));
    rows.iter()
        .enumerate()
        .map(|(i, r)| {
            if i == sel {
                format!("\x1b[7m> {r}\x1b[0m")
            } else {
                format!("  {r}")
            }
        })
        .collect()
}

/// Prints the rows. On a redraw it first moves back up over the rows it printed before.
/// The last row has no newline, so the popup never scrolls.
fn draw(out: &mut impl Write, req: &Request, sel: usize, redraw: bool) {
    let rows = choice_rows(req, sel);
    if redraw {
        let _ = write!(out, "\x1b[{}A\r", rows.len() - 1);
    }
    let _ = write!(out, "\x1b[2K{}", rows.join("\r\n\x1b[2K"));
    let _ = out.flush();
}

/// Popup height: every printed row, with long lines wrapped, plus the border.
#[must_use]
pub fn popup_rows(prompt: &Prompt) -> usize {
    let mut lines = screen(prompt);
    lines.extend(choice_rows(&prompt.request, 0));
    (lines.iter().map(|l| wrapped_rows(l)).sum::<usize>() + 2).min(MAX_ROWS)
}

fn wrapped_rows(line: &str) -> usize {
    line.chars().count().div_ceil(POPUP_WIDTH - 2).max(1)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Key {
    Up,
    Down,
    Enter,
    Digit(u8),
    Cancel,
    Other,
}

fn next_key(b: &mut impl Iterator<Item = u8>) -> Option<Key> {
    Some(match b.next()? {
        b'\r' | b'\n' => Key::Enter,
        b'k' => Key::Up,
        b'j' => Key::Down,
        // Ctrl-C arrives as a byte in raw mode.
        3 => Key::Cancel,
        c @ b'1'..=b'9' => Key::Digit(c - b'0'),
        0x1b => match b.next() {
            Some(b'[' | b'O') => match b.next() {
                Some(b'A') => Key::Up,
                Some(b'B') => Key::Down,
                _ => Key::Other,
            },
            _ => Key::Other,
        },
        _ => Key::Other,
    })
}

#[derive(Debug, PartialEq, Eq)]
enum Step {
    Pick(usize),
    Cancel,
    Redraw,
    Ignore,
}

/// Row 0 is the placeholder. Rows 1..=n are the choices.
struct Picker {
    n: usize,
    sel: usize,
}

impl Picker {
    fn new(n: usize) -> Self {
        Self { n, sel: 0 }
    }

    fn key(&mut self, k: Key) -> Step {
        let before = self.sel;
        match k {
            Key::Up => self.sel = self.sel.saturating_sub(1),
            Key::Down => self.sel = (self.sel + 1).min(self.n),
            Key::Enter if self.sel > 0 => return Step::Pick(self.sel - 1),
            Key::Digit(d) if (1..=self.n).contains(&usize::from(d)) => self.sel = usize::from(d),
            Key::Cancel => return Step::Cancel,
            _ => {}
        }
        if self.sel == before {
            Step::Ignore
        } else {
            Step::Redraw
        }
    }
}

#[cfg(unix)]
mod raw {
    use rustix::termios::{OptionalActions, Termios, isatty, tcgetattr, tcsetattr};
    use std::io::Write;

    /// Raw mode with a hidden cursor, restored on drop. Does nothing when stdin is not a tty.
    pub struct Raw(Termios);

    impl Raw {
        pub fn enter() -> Option<Self> {
            let fd = std::io::stdin();
            if !isatty(&fd) {
                return None;
            }
            let saved = tcgetattr(&fd).ok()?;
            let mut raw = saved.clone();
            raw.make_raw();
            tcsetattr(&fd, OptionalActions::Now, &raw).ok()?;
            let _ = write!(std::io::stdout(), "\x1b[?25l");
            Some(Self(saved))
        }
    }

    impl Drop for Raw {
        fn drop(&mut self) {
            let _ = write!(std::io::stdout(), "\x1b[?25h");
            let _ = std::io::stdout().flush();
            let _ = tcsetattr(std::io::stdin(), OptionalActions::Now, &self.0);
        }
    }
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
            tmux: None,
        }
    }

    /// Rows of what `run` prints before input, counted from the text itself.
    fn printed_rows(p: &Prompt) -> usize {
        let mut out = Vec::new();
        for l in screen(p) {
            out.extend_from_slice(format!("{l}\r\n").as_bytes());
        }
        draw(&mut out, &p.request, 0, false);
        String::from_utf8(out)
            .unwrap()
            .split("\r\n")
            .map(|l| wrapped_rows(&l.replace("\x1b[2K", "")))
            .sum()
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
    fn screen_shows_tmux_location_after_cwd() {
        let mut p = path_prompt("echo hi");
        p.tmux = Some("work:2 (claude)".into());
        let s = screen(&p);
        assert_eq!(s[3], "  in /Users/me/dev/proj");
        assert_eq!(s[4], "  tmux: work:2 (claude)");
        assert_eq!(popup_rows(&p), printed_rows(&p) + 2);
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
            tmux: None,
        };
        let s = screen(&p);
        assert_eq!(s[0], "  node");
        assert_eq!(s[1], "  command: node scripts/build.js --prod");
        assert_eq!(s[3], "  in /Users/me/dev/proj");
    }

    #[test]
    fn placeholder_starts_selected() {
        let rows = choice_rows(&path_prompt("x").request, 0);
        assert!(rows[0].starts_with("\x1b[7m> Pick a selection"));
        assert_eq!(rows[1], "  1. Allow once");
        assert_eq!(rows.len(), 5);
    }

    #[test]
    fn keys_decode() {
        use Key::*;
        let mut b = b"\x1b[A\x1b[B\x1bOAjk\r\n3\x03x".iter().copied();
        let keys: Vec<_> = std::iter::from_fn(|| next_key(&mut b)).collect();
        assert_eq!(
            keys,
            [
                Up,
                Down,
                Up,
                Down,
                Up,
                Enter,
                Enter,
                Digit(3),
                Cancel,
                Other
            ]
        );
    }

    #[test]
    fn enter_on_placeholder_does_nothing() {
        let mut p = Picker::new(4);
        assert_eq!(p.key(Key::Enter), Step::Ignore);
        assert_eq!(p.key(Key::Up), Step::Ignore);
        assert_eq!(p.key(Key::Down), Step::Redraw);
        assert_eq!(p.key(Key::Up), Step::Redraw);
        assert_eq!(p.key(Key::Enter), Step::Ignore);
    }

    #[test]
    fn arrows_then_enter_pick() {
        let mut p = Picker::new(4);
        for _ in 0..6 {
            p.key(Key::Down);
        }
        // Clamped at the last choice, no wrap to the top.
        assert_eq!(p.sel, 4);
        p.key(Key::Up);
        assert_eq!(p.key(Key::Enter), Step::Pick(2));
    }

    #[test]
    fn digits_move_then_enter_picks() {
        let mut p = Picker::new(4);
        assert_eq!(p.key(Key::Digit(4)), Step::Redraw);
        assert_eq!(p.key(Key::Digit(4)), Step::Ignore);
        assert_eq!(p.key(Key::Digit(5)), Step::Ignore);
        assert_eq!(p.key(Key::Digit(1)), Step::Redraw);
        assert_eq!(p.key(Key::Enter), Step::Pick(0));
        assert_eq!(p.key(Key::Cancel), Step::Cancel);
    }
}
