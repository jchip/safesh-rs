//! Command-spec table and wrapper rules (the Interpret stage).
//!
//! Turns one argv into typed items: the program to run, the paths it reads or writes, and
//! reasons to always ask. Command knowledge lives only here, as data.

use crate::effect::Access;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// A program to run, as written (`git`, `./tool`, `/usr/bin/env`). Wrappers yield their inner program too.
    Run(String),
    /// A path argument, relative to the exec cwd unless absolute.
    Path { path: String, access: Access },
    /// Must ask whatever the lists say. Holds a short reason for the prompt.
    Ask(String),
}

/// Interpret one argv (`argv[0]` is the program as written) into run, path and ask items.
#[must_use]
pub fn interpret(argv: &[String]) -> Vec<Item> {
    run(argv, false).items
}

#[derive(Default)]
struct Out {
    items: Vec<Item>,
    /// A known writer was seen somewhere in the chain.
    writer: bool,
}

impl Out {
    fn write(&mut self, path: impl Into<String>) {
        self.items.push(Item::Path {
            path: path.into(),
            access: Access::Write,
        });
        self.writer = true;
    }

    fn write_all(&mut self, paths: &[String]) {
        for p in paths {
            self.write(p.clone());
        }
    }

    fn read(&mut self, path: impl Into<String>) {
        self.items.push(Item::Path {
            path: path.into(),
            access: Access::Read,
        });
    }

    fn ask(&mut self, reason: String) {
        self.items.push(Item::Ask(reason));
    }

    fn unreadable(&mut self, cmd: &str) {
        self.writer = true;
        self.ask(format!("{cmd}: cannot read its arguments"));
    }

    fn inner(&mut self, argv: &[String]) {
        let inner = run(argv, true);
        self.items.extend(inner.items);
        self.writer |= inner.writer;
    }
}

fn base(prog: &str) -> &str {
    prog.rsplit('/').next().unwrap_or(prog)
}

fn join(dir: &str, path: &str) -> String {
    if path.starts_with('/') {
        path.to_string()
    } else {
        format!("{}/{path}", dir.trim_end_matches('/'))
    }
}

// ---------------------------------------------------------------- option parser

/// Option syntax of one command.
#[derive(Default)]
struct Syn {
    /// Short flags that take a value: the rest of the cluster, else the next arg.
    short_val: &'static str,
    /// Short flags whose attached rest is the value and never take the next arg (`-i.bak`).
    short_end: &'static str,
    /// Long flags that take a value, as `--name=v` or `--name v`. Unique prefixes match.
    long_val: &'static [&'static str],
    /// Options end at the first operand (wrappers).
    stop: bool,
    /// Args like `-x` or `-w,a+x` are chmod modes, so they are operands.
    modeish: bool,
}

struct Opt {
    /// `-x` or `--name`.
    name: String,
    val: Option<String>,
}

#[derive(Default)]
struct Parsed {
    opts: Vec<Opt>,
    operands: Vec<String>,
}

impl Parsed {
    fn has(&self, name: &str) -> bool {
        self.opts.iter().any(|o| o.name == name)
    }

    /// Long flag given in full or as an abbreviation (three or more characters).
    fn has_long(&self, full: &str) -> bool {
        self.opts.iter().any(|o| {
            o.name.len() >= 5 && o.name.starts_with("--") && full.starts_with(&o.name[2..])
        })
    }

    fn vals(&self, names: &[&str]) -> Vec<&str> {
        self.opts
            .iter()
            .filter(|o| names.contains(&o.name.as_str()))
            .filter_map(|o| o.val.as_deref())
            .collect()
    }

    fn last(&self, names: &[&str]) -> Option<&str> {
        self.vals(names).last().copied()
    }
}

fn canon_long(name: &str, longs: &[&str]) -> String {
    if longs.contains(&name) || name.is_empty() {
        return name.to_string();
    }
    let hits: Vec<_> = longs.iter().filter(|l| l.starts_with(name)).collect();
    if let [one] = hits.as_slice() {
        return (**one).to_string();
    }
    name.to_string()
}

fn is_mode(arg: &str) -> bool {
    arg.len() > 1
        && arg.starts_with('-')
        && arg[1..]
            .chars()
            .all(|c| "rwxXstugoa,=+-01234567".contains(c))
}

/// `None` when a flag lacks its value.
fn parse(args: &[String], syn: &Syn) -> Option<Parsed> {
    let mut p = Parsed::default();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        i += 1;
        if a == "--" {
            p.operands.extend(args[i..].iter().cloned());
            return Some(p);
        }
        if let Some(body) = a.strip_prefix("--") {
            let (raw, val) = match body.split_once('=') {
                Some((key, value)) => (key, Some(value.to_string())),
                None => (body, None),
            };
            let name = canon_long(raw, syn.long_val);
            let val = if val.is_none() && syn.long_val.contains(&name.as_str()) {
                let v = args.get(i)?.clone();
                i += 1;
                Some(v)
            } else {
                val
            };
            p.opts.push(Opt {
                name: format!("--{name}"),
                val,
            });
            continue;
        }
        if a.len() > 1 && a.starts_with('-') && !(syn.modeish && is_mode(a)) {
            let body = &a[1..];
            for (j, c) in body.char_indices() {
                let rest = &body[j + c.len_utf8()..];
                let name = format!("-{c}");
                if syn.short_val.contains(c) {
                    let v = if rest.is_empty() {
                        let v = args.get(i)?.clone();
                        i += 1;
                        v
                    } else {
                        rest.to_string()
                    };
                    p.opts.push(Opt { name, val: Some(v) });
                    break;
                } else if syn.short_end.contains(c) {
                    let val = (!rest.is_empty()).then(|| rest.to_string());
                    p.opts.push(Opt { name, val });
                    break;
                }
                p.opts.push(Opt { name, val: None });
            }
            continue;
        }
        if syn.stop {
            p.operands.extend(args[i - 1..].iter().cloned());
            return Some(p);
        }
        p.operands.push(a.clone());
    }
    Some(p)
}

// ---------------------------------------------------------------- dispatch

/// `nested` is true for a command started by a wrapper (`env`, `xargs`, `find -exec`...).
/// Such a shell is a real exec outside safesh, so it gets a run check like any program.
fn run(full: &[String], nested: bool) -> Out {
    let mut out = Out::default();
    let Some(prog) = full.first() else {
        return out;
    };
    let name = base(prog);
    let args = &full[1..];
    if nested || !matches!(name, "bash" | "sh" | "dash") {
        out.items.push(Item::Run(prog.clone()));
    }
    let ok = match name {
        "bash" | "sh" | "dash" => {
            shell(args, &mut out);
            Some(())
        }
        "rm" | "rmdir" | "touch" | "mkdir" | "truncate" | "shred" | "tee" => {
            all_operands(name, args, &mut out)
        }
        "mv" | "cp" | "install" | "ln" => to_dest(name, args, &mut out),
        "rsync" => rsync(args, &mut out),
        "chmod" | "chown" | "chgrp" => owner_mode(name, args, &mut out),
        "sed" => sed(args, &mut out),
        "perl" => perl(args, &mut out),
        "dd" => {
            out.writer = true;
            for a in args {
                if let Some(of) = a.strip_prefix("of=") {
                    out.write(of);
                }
            }
            Some(())
        }
        "tar" => tar(args, &mut out),
        "unzip" => unzip(args, &mut out),
        "curl" => curl(args, &mut out),
        "wget" => wget(args, &mut out),
        "find" => find(args, &mut out),
        "git" => {
            git(args, &mut out);
            Some(())
        }
        "env" => env(args, &mut out),
        "nice" | "nohup" | "time" | "timeout" | "stdbuf" => simple_wrapper(name, args, &mut out),
        "xargs" => xargs(args, &mut out),
        "sudo" | "doas" => sudo(name, args, &mut out),
        _ => Some(()),
    };
    if ok.is_none() {
        out.unreadable(name);
    }
    out
}

// ---------------------------------------------------------------- shells

/// `bash script.sh` reads the script. `-c` payloads are checked by the nested safesh.
fn shell(args: &[String], out: &mut Out) {
    let (mut c, mut s) = (false, false);
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        i += 1;
        if a == "--" {
            break;
        }
        if !(a.starts_with('-') || a.starts_with('+')) || a.len() < 2 {
            i -= 1;
            break;
        }
        if matches!(a.as_str(), "--rcfile" | "--init-file") {
            i += 1;
            continue;
        }
        if a.starts_with("--") {
            continue;
        }
        for ch in a[1..].chars() {
            match ch {
                'c' => c = true,
                's' => s = true,
                'o' | 'O' => i += 1,
                _ => {}
            }
        }
    }
    if !c
        && !s
        && let Some(script) = args.get(i)
    {
        out.read(script.clone());
    }
}

// ---------------------------------------------------------------- simple writers

fn syn_all(name: &str) -> Syn {
    match name {
        "touch" => Syn {
            short_val: "drtA",
            long_val: &["date", "reference", "time"],
            ..Syn::default()
        },
        "mkdir" => Syn {
            short_val: "m",
            long_val: &["mode"],
            ..Syn::default()
        },
        "truncate" => Syn {
            short_val: "sr",
            long_val: &["size", "reference"],
            ..Syn::default()
        },
        "shred" => Syn {
            short_val: "ns",
            long_val: &["iterations", "size", "random-source"],
            ..Syn::default()
        },
        _ => Syn::default(),
    }
}

fn all_operands(name: &str, args: &[String], out: &mut Out) -> Option<()> {
    let p = parse(args, &syn_all(name))?;
    out.writer = true;
    for o in p.operands {
        out.write(o);
    }
    Some(())
}

fn to_dest(name: &str, args: &[String], out: &mut Out) -> Option<()> {
    let syn = Syn {
        short_val: if name == "install" { "tSmogBf" } else { "tS" },
        long_val: &[
            "target-directory",
            "suffix",
            "mode",
            "owner",
            "group",
            "strip-program",
        ],
        ..Syn::default()
    };
    let p = parse(args, &syn)?;
    let ops = &p.operands;
    out.writer = true;
    if name == "install" && (p.has("-d") || p.has_long("directory")) {
        if ops.is_empty() {
            return None;
        }
        out.write_all(ops);
    } else if let Some(t) = p.last(&["-t", "--target-directory"]) {
        out.write(t);
        if name == "mv" {
            out.write_all(ops);
        }
    } else if name == "ln" {
        match ops.len() {
            0 => return None,
            1 => out.write("."),
            _ => out.write(ops[ops.len() - 1].clone()),
        }
    } else if ops.len() < 2 {
        return None;
    } else if name == "mv" {
        out.write_all(ops);
    } else {
        out.write(ops[ops.len() - 1].clone());
    }
    Some(())
}

fn is_remote(s: &str) -> bool {
    s.starts_with("rsync://") || s.find(':').is_some_and(|c| c > 0 && !s[..c].contains('/'))
}

fn rsync(args: &[String], out: &mut Out) -> Option<()> {
    let syn = Syn {
        short_val: "efBTM@",
        long_val: &[
            "rsh",
            "exclude",
            "include",
            "exclude-from",
            "include-from",
            "files-from",
            "filter",
            "port",
            "bwlimit",
            "timeout",
            "contimeout",
            "rsync-path",
            "backup-dir",
            "suffix",
            "temp-dir",
            "partial-dir",
            "log-file",
            "log-file-format",
            "compare-dest",
            "copy-dest",
            "link-dest",
            "chmod",
            "chown",
            "usermap",
            "groupmap",
            "max-size",
            "min-size",
            "block-size",
            "modify-window",
            "password-file",
            "out-format",
            "info",
            "debug",
            "remote-option",
            "protocol",
            "iconv",
            "address",
            "sockopts",
            "write-batch",
            "read-batch",
            "only-write-batch",
            "checksum-choice",
            "compress-choice",
            "compress-level",
            "skip-compress",
            "max-delete",
            "stop-after",
            "stop-at",
        ],
        ..Syn::default()
    };
    let p = parse(args, &syn)?;
    out.writer = true;
    if p.operands.len() >= 2 {
        let dest = &p.operands[p.operands.len() - 1];
        if !is_remote(dest) {
            out.write(dest.clone());
        }
    }
    Some(())
}

fn owner_mode(name: &str, args: &[String], out: &mut Out) -> Option<()> {
    let syn = Syn {
        long_val: if name == "chown" {
            &["reference", "from"]
        } else {
            &["reference"]
        },
        modeish: name == "chmod",
        ..Syn::default()
    };
    let p = parse(args, &syn)?;
    out.writer = true;
    let skip = usize::from(p.last(&["--reference"]).is_none());
    for o in p.operands.iter().skip(skip) {
        out.write(o.clone());
    }
    Some(())
}

// ---------------------------------------------------------------- sed, perl, dd

fn sed(args: &[String], out: &mut Out) -> Option<()> {
    // BSD `-i ''`: the empty arg is the backup suffix.
    let mut v: Vec<String> = Vec::new();
    for (k, a) in args.iter().enumerate() {
        let after_bsd_i = k > 0
            && args[k - 1].starts_with('-')
            && !args[k - 1].starts_with("--")
            && args[k - 1].ends_with(['i', 'I'])
            && a.is_empty();
        if !after_bsd_i {
            v.push(a.clone());
        }
    }
    let syn = Syn {
        short_val: "efl",
        short_end: "iI",
        long_val: &["expression", "file", "line-length"],
        ..Syn::default()
    };
    let p = parse(&v, &syn)?;
    let in_place = p.has("-i") || p.has("-I") || p.has_long("in-place");
    if !in_place {
        return Some(());
    }
    out.writer = true;
    let scripted = p.has("-e") || p.has("-f") || p.has("--expression") || p.has("--file");
    if !scripted && p.operands.is_empty() {
        return None;
    }
    for f in p.operands.iter().skip(usize::from(!scripted)) {
        out.write(f.clone());
    }
    Some(())
}

// `-pie` means in-place with suffix `e`, as in perl itself.
fn perl(args: &[String], out: &mut Out) -> Option<()> {
    let (mut in_place, mut scripted) = (false, false);
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        i += 1;
        if a == "--" {
            break;
        }
        if !a.starts_with('-') || a.len() < 2 {
            i -= 1;
            break;
        }
        let body = &a[1..];
        let mut chars = body.char_indices();
        while let Some((j, c)) = chars.next() {
            let rest = &body[j + c.len_utf8()..];
            match c {
                'i' => {
                    in_place = true;
                    break;
                }
                'e' | 'E' => {
                    scripted = true;
                    if rest.is_empty() {
                        args.get(i)?;
                        i += 1;
                    }
                    break;
                }
                'I' => {
                    if rest.is_empty() {
                        args.get(i)?;
                        i += 1;
                    }
                    break;
                }
                'M' | 'm' | 'F' | 'x' | 'C' | 'd' | 'D' | 'V' => break,
                // `-l` and `-0` take only digits, then the cluster goes on.
                'l' | '0' => {
                    let digits = rest.chars().take_while(char::is_ascii_digit).count();
                    for _ in 0..digits {
                        chars.next();
                    }
                }
                _ => {}
            }
        }
    }
    if in_place {
        out.writer = true;
        for f in args.iter().skip(i + usize::from(!scripted)) {
            out.write(f.clone());
        }
    }
    Some(())
}

// ---------------------------------------------------------------- tar, unzip, curl, wget

const TAR_VAL: &str = "fCTXbLNIHVgK";

fn tar(args: &[String], out: &mut Out) -> Option<()> {
    let mut v: Vec<String> = Vec::new();
    // Old style `tar xzf a.tgz`: each value letter takes the next operand in order.
    if let Some(first) = args.first().filter(|a| !a.starts_with('-')) {
        let mut vals = args[1..].iter();
        let mut used = 0;
        for c in first.chars() {
            v.push(format!("-{c}"));
            if TAR_VAL.contains(c) {
                v.push(vals.next()?.clone());
                used += 1;
            }
        }
        v.extend(args[1 + used..].iter().cloned());
    } else {
        v.extend(args.iter().cloned());
    }
    let syn = Syn {
        short_val: TAR_VAL,
        long_val: &[
            "file",
            "directory",
            "files-from",
            "exclude-from",
            "exclude",
            "use-compress-program",
            "strip-components",
            "transform",
            "xform",
            "to-command",
            "format",
            "label",
            "listed-incremental",
            "starting-file",
            "blocking-factor",
            "newer",
            "newer-mtime",
            "owner",
            "group",
            "mode",
            "mtime",
            "suffix",
            "cd",
            "include",
            "options",
            "gid",
            "uid",
            "gname",
            "uname",
            "checkpoint-action",
            "rsh-command",
            "volno-file",
            "index-file",
            "tape-length",
            "pax-option",
        ],
        ..Syn::default()
    };
    let p = parse(&v, &syn)?;
    if !(p.has("-x") || p.has_long("extract") || p.has_long("get")) {
        return Some(());
    }
    let dirs = p.vals(&["-C", "--directory", "--cd"]);
    if dirs.is_empty() {
        out.write(".");
    }
    for d in dirs {
        out.write(d);
    }
    Some(())
}

fn unzip(args: &[String], out: &mut Out) -> Option<()> {
    let syn = Syn {
        short_val: "dxP",
        ..Syn::default()
    };
    let p = parse(args, &syn)?;
    // -l list, -t test, -p to stdout, -Z zipinfo, -v verbose list.
    if ["-l", "-t", "-p", "-Z", "-v"].iter().any(|f| p.has(f)) {
        return Some(());
    }
    out.write(p.last(&["-d"]).unwrap_or("."));
    Some(())
}

fn skip_dash(v: &str) -> bool {
    v == "-"
}

fn curl(args: &[String], out: &mut Out) -> Option<()> {
    let syn = Syn {
        short_val: "AbcCdDeEFHKmoPrtTuUwxXyYzQ",
        long_val: &[
            "output",
            "dump-header",
            "cookie-jar",
            "header",
            "data",
            "data-raw",
            "data-binary",
            "data-urlencode",
            "data-ascii",
            "user",
            "user-agent",
            "request",
            "url",
            "form",
            "referer",
            "proxy",
            "max-time",
            "connect-timeout",
            "retry",
            "retry-delay",
            "retry-max-time",
            "cacert",
            "cert",
            "key",
            "upload-file",
            "range",
            "write-out",
            "config",
            "output-dir",
            "trace",
            "trace-ascii",
            "cookie",
            "oauth2-bearer",
            "resolve",
            "connect-to",
            "interface",
            "limit-rate",
            "proxy-user",
            "continue-at",
            "capath",
            "json",
            "etag-save",
            "etag-compare",
            "proto",
            "tlsv1.3",
        ],
        ..Syn::default()
    };
    let p = parse(args, &syn)?;
    let dir = p.last(&["--output-dir"]);
    for f in p.vals(&["-o", "--output"]) {
        if !skip_dash(f) {
            out.write(dir.map_or_else(|| f.to_string(), |d| join(d, f)));
        }
    }
    for f in p.vals(&[
        "-D",
        "--dump-header",
        "-c",
        "--cookie-jar",
        "--trace",
        "--trace-ascii",
    ]) {
        if !skip_dash(f) {
            out.write(f);
        }
    }
    // The remote basename lands in cwd (or --output-dir).
    if p.has("-O") || p.has_long("remote-name") || p.has_long("remote-name-all") {
        out.write(dir.unwrap_or("."));
    }
    Some(())
}

fn wget(args: &[String], out: &mut Out) -> Option<()> {
    let syn = Syn {
        short_val: "OPoaetTwQiBUlARDIX",
        short_end: "n",
        long_val: &[
            "output-document",
            "directory-prefix",
            "output-file",
            "append-output",
            "execute",
            "tries",
            "timeout",
            "wait",
            "user-agent",
            "header",
            "post-data",
            "post-file",
            "input-file",
            "save-cookies",
            "load-cookies",
            "referer",
            "user",
            "password",
            "http-user",
            "http-password",
            "limit-rate",
            "quota",
            "accept",
            "reject",
            "level",
        ],
        ..Syn::default()
    };
    let p = parse(args, &syn)?;
    match p.last(&["-O", "--output-document"]) {
        Some(f) if skip_dash(f) => {}
        Some(f) => out.write(f),
        None => out.write(p.last(&["-P", "--directory-prefix"]).unwrap_or(".")),
    }
    for f in p.vals(&[
        "-o",
        "--output-file",
        "-a",
        "--append-output",
        "--save-cookies",
    ]) {
        out.write(f);
    }
    Some(())
}

// ---------------------------------------------------------------- find, git

const FIND_VAL: &[&str] = &[
    "-name",
    "-iname",
    "-path",
    "-ipath",
    "-wholename",
    "-iwholename",
    "-regex",
    "-iregex",
    "-lname",
    "-ilname",
    "-type",
    "-xtype",
    "-user",
    "-group",
    "-uid",
    "-gid",
    "-perm",
    "-size",
    "-mtime",
    "-atime",
    "-ctime",
    "-mmin",
    "-amin",
    "-cmin",
    "-newer",
    "-anewer",
    "-cnewer",
    "-maxdepth",
    "-mindepth",
    "-samefile",
    "-printf",
    "-fstype",
    "-links",
    "-inum",
    "-regextype",
    "-used",
    "-context",
];

fn find(args: &[String], out: &mut Out) -> Option<()> {
    let mut roots: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-H" | "-L" | "-P" | "-E" | "-X" | "-d" | "-s" | "-x" => i += 1,
            "-D" => i += 2,
            // BSD `-f path` names a root.
            "-f" => {
                roots.extend(args.get(i + 1).cloned());
                i += 2;
            }
            s if s.starts_with("-O") => i += 1,
            _ => break,
        }
    }
    while i < args.len()
        && !(args[i].starts_with('-') || matches!(args[i].as_str(), "(" | ")" | "!" | ","))
    {
        roots.push(args[i].clone());
        i += 1;
    }
    if roots.is_empty() {
        roots.push(".".to_string());
    }
    let mut deleted = false;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "-delete" => {
                if !deleted {
                    out.write_all(&roots);
                    deleted = true;
                }
                i += 1;
            }
            "-exec" | "-execdir" | "-ok" | "-okdir" => {
                let mut j = i + 1;
                loop {
                    let t = args.get(j)?;
                    if t == ";" || (t == "+" && j > i + 1 && args[j - 1] == "{}") {
                        break;
                    }
                    j += 1;
                }
                let cmd = &args[i + 1..j];
                if cmd.is_empty() {
                    return None;
                }
                let inner = run(cmd, true);
                out.writer |= inner.writer;
                for item in inner.items {
                    match item {
                        Item::Path { path, access } if path.contains("{}") => {
                            for r in &roots {
                                out.items.push(Item::Path {
                                    path: r.clone(),
                                    access,
                                });
                            }
                        }
                        other => out.items.push(other),
                    }
                }
                i = j + 1;
            }
            "-fprint" | "-fprint0" | "-fls" | "-fprintf" => {
                out.write(args.get(i + 1)?.clone());
                i += if a == "-fprintf" { 3 } else { 2 };
            }
            _ if FIND_VAL.contains(&a) || (a.starts_with("-newer") && a.len() == 8) => i += 2,
            _ => i += 1,
        }
    }
    Some(())
}

fn git(args: &[String], out: &mut Out) {
    let mut dir: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-C" => {
                if let Some(d) = args.get(i + 1) {
                    dir = Some(dir.map_or_else(|| d.clone(), |cur| join(&cur, d)));
                }
                i += 2;
            }
            "-c" | "--git-dir" | "--work-tree" | "--namespace" | "--config-env"
            | "--super-prefix" | "--attr-source" => i += 2,
            s if s.starts_with('-') => i += 1,
            _ => break,
        }
    }
    if args.get(i).is_some_and(|s| s == "clean") {
        out.write(dir.unwrap_or_else(|| ".".to_string()));
    }
}

// ---------------------------------------------------------------- wrappers

fn env(args: &[String], out: &mut Out) -> Option<()> {
    let syn = Syn {
        short_val: "uCPSa",
        long_val: &["unset", "chdir", "split-string", "argv0"],
        stop: true,
        ..Syn::default()
    };
    let p = parse(args, &syn)?;
    if p.has("-S") || p.has("--split-string") {
        out.ask("env: cannot read its arguments".to_string());
        return Some(());
    }
    let mut rest = &p.operands[..];
    if rest.first().is_some_and(|a| a == "-") {
        rest = &rest[1..];
    }
    while rest
        .first()
        .is_some_and(|a| a.contains('=') && !a.starts_with('='))
    {
        rest = &rest[1..];
    }
    if rest.is_empty() {
        return Some(());
    }
    let mut inner = run(rest, true);
    if let Some(dir) = p.last(&["-C", "--chdir"]) {
        for item in &mut inner.items {
            if let Item::Path { path, .. } = item {
                *path = join(dir, path);
            }
        }
    }
    out.items.extend(inner.items);
    out.writer |= inner.writer;
    Some(())
}

fn simple_wrapper(name: &str, args: &[String], out: &mut Out) -> Option<()> {
    let syn = match name {
        "nice" => Syn {
            short_val: "n",
            long_val: &["adjustment"],
            stop: true,
            ..Syn::default()
        },
        "time" => Syn {
            short_val: "fo",
            long_val: &["format", "output"],
            stop: true,
            ..Syn::default()
        },
        "timeout" => Syn {
            short_val: "ks",
            long_val: &["kill-after", "signal"],
            stop: true,
            ..Syn::default()
        },
        "stdbuf" => Syn {
            short_val: "ioe",
            long_val: &["input", "output", "error"],
            stop: true,
            ..Syn::default()
        },
        _ => Syn {
            stop: true,
            ..Syn::default()
        },
    };
    let p = parse(args, &syn)?;
    if name == "time" {
        for f in p.vals(&["-o", "--output"]) {
            out.write(f);
        }
    }
    // timeout takes DURATION before the command.
    let skip = usize::from(name == "timeout");
    if p.operands.len() > skip {
        out.inner(&p.operands[skip..]);
    }
    Some(())
}

fn xargs(args: &[String], out: &mut Out) -> Option<()> {
    let syn = Syn {
        short_val: "ILnPsdEaJRS",
        short_end: "ile",
        long_val: &[
            "arg-file",
            "delimiter",
            "max-args",
            "max-procs",
            "max-lines",
            "max-chars",
            "process-slot-var",
        ],
        stop: true,
        ..Syn::default()
    };
    let p = parse(args, &syn)?;
    let cmd = if p.operands.is_empty() {
        vec!["echo".to_string()]
    } else {
        p.operands
    };
    let mut inner = run(&cmd, true);
    if inner.writer {
        inner.items.retain(|i| !matches!(i, Item::Path { .. }));
        inner.items.push(Item::Ask(format!(
            "xargs {}: paths come from input",
            base(&cmd[0])
        )));
    }
    out.items.extend(inner.items);
    out.writer |= inner.writer;
    Some(())
}

fn sudo(name: &str, args: &[String], out: &mut Out) -> Option<()> {
    out.ask(format!("{name} runs as another user"));
    let syn = Syn {
        short_val: "uCDgGhpPrtTUc",
        long_val: &[
            "user",
            "group",
            "close-from",
            "chdir",
            "host",
            "prompt",
            "role",
            "type",
            "command-timeout",
            "other-user",
            "chroot",
        ],
        stop: true,
        ..Syn::default()
    };
    let p = parse(args, &syn)?;
    // Shell, edit, list and validate modes name no command.
    if ["-i", "-s", "-e", "-l", "-V", "-v", "-K"]
        .iter()
        .any(|f| p.has(f))
        || ["login", "shell", "edit", "list"]
            .iter()
            .any(|f| p.has_long(f))
    {
        return Some(());
    }
    let mut rest = &p.operands[..];
    while rest
        .first()
        .is_some_and(|a| a.contains('=') && !a.starts_with('='))
    {
        rest = &rest[1..];
    }
    if !rest.is_empty() {
        out.inner(rest);
    }
    Some(())
}
