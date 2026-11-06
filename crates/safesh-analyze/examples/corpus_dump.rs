//! Reads `{"id","cmd"}` JSONL (argv[1]) and writes one analysis summary per line (argv[2]).
//! Never executes anything.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;

use safesh_analyze::{Access, AnalyzeOptions, OpenKind, Word, analyze};
use serde_json::{Value, json};

fn summarize(cmd: &str, opts: &AnalyzeOptions) -> Value {
    match analyze(cmd, opts) {
        Err(e) => json!({"parse_ok": false, "error": e.to_string()}),
        Ok(a) => {
            let mut commands = BTreeSet::new();
            let mut builtins = BTreeSet::new();
            let mut unknown = 0;
            for e in &a.execs {
                match &e.argv[0] {
                    Word::Unknown(_) => unknown += 1,
                    Word::Known(n) if e.builtin => {
                        builtins.insert(n.clone());
                    }
                    Word::Known(n) if !e.function => {
                        commands.insert(n.clone());
                    }
                    Word::Known(_) => {}
                }
            }
            let redirects: Vec<Value> = a
                .opens
                .iter()
                .filter(|o| o.kind == OpenKind::Redirect)
                .filter_map(|o| match &o.path {
                    Word::Known(p) => Some(json!({
                        "path": p,
                        "operation": if o.access == Access::Read { "read" } else { "write" },
                    })),
                    Word::Unknown(_) => None,
                })
                .collect();
            json!({
                "parse_ok": true,
                "commands": commands,
                "builtins": builtins,
                "unknown_commands": unknown,
                "redirects": redirects,
                "finalCwd": a.final_cwd.as_ref().map(|p| p.to_string_lossy().into_owned()),
            })
        }
    }
}

fn run(input: &str, output: &str) -> std::io::Result<()> {
    let opts = AnalyzeOptions {
        base_cwd: PathBuf::from("/base"),
        env: BTreeMap::from([("HOME".to_string(), "/home/u".to_string())]),
    };
    let mut out = BufWriter::new(std::fs::File::create(output)?);
    for line in BufReader::new(std::fs::File::open(input)?).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let id = v.get("id").cloned().unwrap_or(Value::Null);
        let cmd = v.get("cmd").and_then(Value::as_str).unwrap_or("");
        let res = panic::catch_unwind(AssertUnwindSafe(|| summarize(cmd, &opts)))
            .unwrap_or_else(|_| json!({"parse_ok": false, "error": "panic"}));
        let mut obj = res.as_object().cloned().unwrap_or_default();
        obj.insert("id".to_string(), id);
        writeln!(out, "{}", Value::Object(obj))?;
    }
    out.flush()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: corpus_dump <in.jsonl> <out.jsonl>");
        std::process::exit(2);
    }
    let (i, o) = (args[1].clone(), args[2].clone());
    // Deep nesting needs more stack than the default main thread has.
    let h = std::thread::Builder::new()
        .stack_size(512 << 20)
        .spawn(move || run(&i, &o))
        .expect("spawn");
    if let Err(e) = h.join().expect("worker") {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
