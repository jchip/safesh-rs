//! `safesh approve-ui`: the picker shown inside the tmux popup.
//!
//! It reads the prompt file, asks for a number, and writes the answer file. At the deadline,
//! or on end of input, it exits without an answer, which the caller treats as no answer.

use std::io::{BufRead, Write};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::{body_lines, files, numbered_labels};

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
    let _ = writeln!(out, "{}\n", req.title());
    for l in body_lines(&pf.prompt) {
        let _ = writeln!(out, "  {l}");
    }
    let _ = writeln!(out);
    for l in numbered_labels(req) {
        let _ = writeln!(out, "{l}");
    }
    let n = req.choices().len();
    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();
    loop {
        let _ = write!(out, "\nchoice (1-{n}): ");
        let _ = out.flush();
        let Some(Ok(line)) = lines.next() else {
            return 0;
        };
        if let Some(c) = parse(&line, n).map(|i| req.choices()[i]) {
            return u8::from(files::write_answer(state_dir, id, &pf.nonce, c).is_none());
        }
    }
}

fn parse(line: &str, n: usize) -> Option<usize> {
    let i: usize = line.trim().parse().ok()?;
    (1..=n).contains(&i).then(|| i - 1)
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn parse_range() {
        assert_eq!(parse(" 1\n", 5), Some(0));
        assert_eq!(parse("5", 5), Some(4));
        assert_eq!(parse("0", 5), None);
        assert_eq!(parse("5", 4), None);
        assert_eq!(parse("y", 5), None);
    }
}
