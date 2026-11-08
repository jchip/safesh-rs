//! Windows path rules: Git Bash (MSYS) paths, drive and UNC prefixes, case-insensitive matching.
//!
//! Every function here is plain string work, so it builds and is tested on every platform. The
//! `paths` module calls into it only on Windows.
//!
//! Git Bash hands us POSIX spellings that mean something else to Windows. `/c/Users` is `C:\Users`,
//! `/tmp` is the user temp dir, and `/` and `/usr/bin` live under the Git install. Policy and the
//! engine must read each spelling the same way, so both go through [`resolve`].

use std::sync::OnceLock;

/// Where Git Bash's mount points live on this machine.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Mounts {
    /// The Git install, mounted on `/`. `None` when Git Bash isn't found.
    pub root: Option<String>,
    /// The user temp dir, mounted on `/tmp`.
    pub tmp: Option<String>,
}

/// The standard Git for Windows location. It is tried before any env var, since the agent's shell
/// can set those.
const DEFAULT_GIT_ROOT: &str = r"C:\Program Files\Git";

impl Mounts {
    /// `root` is the first Git install that has `usr\bin\msys-2.0.dll`: the standard location, then
    /// `EXEPATH` (`<git>\bin`), then `SHELL` (`<git>\usr\bin\bash.exe`). `tmp` is `TEMP`, then `TMP`.
    pub fn from_lookup(
        get: impl Fn(&str) -> Option<String>,
        is_git: impl Fn(&str) -> bool,
    ) -> Self {
        let from_exepath = get("EXEPATH").and_then(|p| strip_suffix_ci(&normalize(&p), r"\bin"));
        let from_shell =
            get("SHELL").and_then(|p| strip_suffix_ci(&normalize(&p), r"\usr\bin\bash.exe"));
        let root = std::iter::once(Some(DEFAULT_GIT_ROOT.to_string()))
            .chain([from_exepath, from_shell])
            .flatten()
            .find(|r| is_git(r));
        let tmp = get("TEMP")
            .or_else(|| get("TMP"))
            .map(|t| normalize(&t))
            .filter(|t| is_absolute(t));
        Self { root, tmp }
    }
}

/// Mounts for this process, read once from its environment.
pub fn mounts() -> &'static Mounts {
    static MOUNTS: OnceLock<Mounts> = OnceLock::new();
    MOUNTS.get_or_init(|| {
        Mounts::from_lookup(
            |k| std::env::var(k).ok().filter(|v| !v.is_empty()),
            |root| {
                std::path::Path::new(root)
                    .join(r"usr\bin\msys-2.0.dll")
                    .is_file()
            },
        )
    })
}

/// `PATHEXT` for this process, with the Windows default when unset.
pub fn pathext() -> &'static str {
    static PATHEXT: OnceLock<String> = OnceLock::new();
    PATHEXT.get_or_init(|| {
        std::env::var("PATHEXT")
            .ok()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".to_string())
    })
}

fn strip_suffix_ci(s: &str, suffix: &str) -> Option<String> {
    let cut = s.len().checked_sub(suffix.len())?;
    (s.is_char_boundary(cut) && s[cut..].eq_ignore_ascii_case(suffix)).then(|| s[..cut].to_string())
}

fn is_sep(c: char) -> bool {
    c == '/' || c == '\\'
}

/// `X:` followed by a separator.
fn has_drive_root(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && is_sep(b[2] as char)
}

/// Fully qualified: `C:\x`, `\\server\share\x` or `\\?\C:\x`. A rooted path with no drive,
/// such as `\x` or `/x`, is not.
#[must_use]
pub fn is_absolute(path: &str) -> bool {
    has_drive_root(path) || (path.len() > 2 && path.chars().take(2).all(is_sep))
}

/// Git Bash spelling of a rooted path to the Windows path it names. Only paths with exactly one
/// leading `/` are mapped. `None` when the path needs a mount we don't know, and for `/dev`, whose
/// entries are devices, not files under the Git install.
#[must_use]
pub fn msys_to_windows(path: &str, m: &Mounts) -> Option<String> {
    let rest = path.strip_prefix('/').filter(|r| !r.starts_with('/'))?;
    let (first, tail) = rest.split_once('/').unwrap_or((rest, ""));
    let join = |base: &str, tail: &str| {
        let base = base.trim_end_matches(is_sep);
        if tail.is_empty() {
            format!("{base}\\")
        } else {
            format!("{base}\\{tail}")
        }
    };
    if first.len() == 1 && first.as_bytes()[0].is_ascii_alphabetic() {
        return Some(normalize(&join(
            &format!("{}:", first.to_ascii_uppercase()),
            tail,
        )));
    }
    let mapped = match first {
        "dev" => return None,
        "tmp" => join(m.tmp.as_deref()?, tail),
        "bin" => join(&join(m.root.as_deref()?, r"usr\bin"), tail),
        _ => join(m.root.as_deref()?, rest),
    };
    Some(normalize(&mapped))
}

/// Split into prefix (`C:` or `\\server\share`), whether a root separator follows, and the rest.
fn split_prefix(p: &str) -> (String, bool, &str) {
    if let Some(unc) = p.strip_prefix(r"\\") {
        let mut it = unc.splitn(3, '\\');
        let server = it.next().unwrap_or("");
        let share = it.next().unwrap_or("");
        let rest = it.next().unwrap_or("");
        return (format!(r"\\{server}\{share}"), true, rest);
    }
    let b = p.as_bytes();
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        let drive = format!("{}:", (b[0] as char).to_ascii_uppercase());
        let rest = &p[2..];
        return match rest.strip_prefix('\\') {
            Some(r) => (drive, true, r),
            None => (drive, false, rest),
        };
    }
    match p.strip_prefix('\\') {
        Some(r) => (String::new(), true, r),
        None => (String::new(), false, p),
    }
}

/// Lexically normalize a Windows path: `/` becomes `\`, `.` and empty parts go, `..` is resolved
/// without climbing above the root, the drive letter is uppercased and a `\\?\` prefix is dropped.
/// A trailing separator is kept. Relative paths stay relative.
#[must_use]
pub fn normalize(path: &str) -> String {
    if path.is_empty() {
        return ".".to_string();
    }
    let p = path.replace('/', "\\");
    let p = if let Some(r) = p.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{r}")
    } else if let Some(r) = p.strip_prefix(r"\\?\") {
        r.to_string()
    } else {
        p
    };
    let trailing = p.ends_with('\\');
    let (prefix, rooted, rest) = split_prefix(&p);
    let mut parts: Vec<&str> = Vec::new();
    for seg in rest.split('\\') {
        match seg {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|l| *l != "..") {
                    parts.pop();
                } else if !rooted {
                    parts.push("..");
                }
            }
            s => parts.push(s),
        }
    }
    let mut out = prefix;
    if rooted {
        out.push('\\');
    }
    out.push_str(&parts.join("\\"));
    if out.is_empty() {
        out.push('.');
    }
    if trailing && !out.ends_with('\\') {
        out.push('\\');
    }
    out
}

/// The drive or UNC share of an absolute `cwd`, for paths rooted without one.
fn prefix_of(cwd: &str) -> String {
    split_prefix(&normalize(cwd)).0
}

/// Absolute, normalized form of `path` against `cwd`, with no trailing separator except on a root.
/// Git Bash spellings go through `m`. A rooted path with no known mount lands on `cwd`'s drive,
/// as Windows itself would read it.
#[must_use]
pub fn resolve(cwd: &str, path: &str, m: &Mounts) -> String {
    let joined = if is_absolute(path) {
        path.to_string()
    } else if let Some(win) = msys_to_windows(path, m) {
        win
    } else if path.starts_with(is_sep) {
        format!("{}{path}", prefix_of(cwd))
    } else if path.len() >= 2
        && path.as_bytes()[0].is_ascii_alphabetic()
        && path.as_bytes()[1] == b':'
    {
        // `C:foo` is relative to that drive's own cwd, which we can't know. Read it from the root.
        format!("{}\\{}", &path[..2], &path[2..])
    } else {
        format!("{cwd}\\{path}")
    };
    let mut out = normalize(&joined);
    while out.ends_with('\\') && !is_root(&out) {
        out.pop();
    }
    out
}

fn is_root(p: &str) -> bool {
    let (_, rooted, rest) = split_prefix(p);
    rooted && rest.is_empty()
}

/// `path` equals `parent` or sits below it, ignoring ASCII case and separator spelling.
#[must_use]
pub fn is_within(path: &str, parent: &str) -> bool {
    let p = normalize(path).to_ascii_lowercase();
    let q = normalize(parent).to_ascii_lowercase();
    let q = q.trim_end_matches('\\');
    let p = p.trim_end_matches('\\');
    p == q || p.strip_prefix(q).is_some_and(|r| r.starts_with('\\'))
}

/// Same file name, ignoring ASCII case and separator spelling.
#[must_use]
pub fn same_path(a: &str, b: &str) -> bool {
    normalize(a).eq_ignore_ascii_case(&normalize(b))
}

/// The command name in `prog`: the last path part, minus a `PATHEXT` extension. `C:\x\ls.exe` and
/// `ls.EXE` are both `ls`.
#[must_use]
pub fn command_name<'a>(prog: &'a str, pathext: &str) -> &'a str {
    let base = prog.rsplit(is_sep).next().unwrap_or(prog);
    match base.rsplit_once('.') {
        Some((stem, ext))
            if !stem.is_empty()
                && pathext.split(';').any(|e| {
                    !e.is_empty() && e.trim_start_matches('.').eq_ignore_ascii_case(ext)
                }) =>
        {
            stem
        }
        _ => base,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git() -> Mounts {
        Mounts {
            root: Some(r"C:\Program Files\Git".into()),
            tmp: Some(r"C:\Users\me\AppData\Local\Temp".into()),
        }
    }

    #[test]
    fn msys_paths_map_like_cygpath() {
        let m = git();
        let cases = [
            ("/c/Users/me", r"C:\Users\me"),
            ("/d", r"D:\"),
            ("/tmp", r"C:\Users\me\AppData\Local\Temp\"),
            ("/tmp/x.txt", r"C:\Users\me\AppData\Local\Temp\x.txt"),
            ("/usr/bin/ls", r"C:\Program Files\Git\usr\bin\ls"),
            ("/bin/bash", r"C:\Program Files\Git\usr\bin\bash"),
            ("/etc/fstab", r"C:\Program Files\Git\etc\fstab"),
            ("/", r"C:\Program Files\Git\"),
        ];
        for (msys, win) in cases {
            assert_eq!(msys_to_windows(msys, &m).as_deref(), Some(win), "{msys}");
        }
        assert_eq!(msys_to_windows("//server/share", &m), None);
        assert_eq!(msys_to_windows("/dev/null", &m), None);
        assert_eq!(msys_to_windows("rel/x", &m), None);
        assert_eq!(msys_to_windows("/etc/x", &Mounts::default()), None);
        assert_eq!(
            msys_to_windows("/c/x", &Mounts::default()).as_deref(),
            Some(r"C:\x")
        );
    }

    #[test]
    fn normalize_handles_prefixes_and_dots() {
        let cases = [
            (r"C:\a\b\..\c", r"C:\a\c"),
            ("c:/a//b/./c", r"C:\a\b\c"),
            (r"C:\..\..\x", r"C:\x"),
            (r"\\?\C:\a\b", r"C:\a\b"),
            (r"\\?\UNC\srv\share\x", r"\\srv\share\x"),
            (r"\\srv\share\a\..\b", r"\\srv\share\b"),
            (r"a\..\..\b", r"..\b"),
            (r"C:\a\", r"C:\a\"),
            ("", "."),
            (r"C:\", r"C:\"),
        ];
        for (input, want) in cases {
            assert_eq!(normalize(input), want, "{input}");
        }
    }

    #[test]
    fn resolve_reads_every_spelling() {
        let m = git();
        let cwd = r"C:\Users\me\proj";
        let cases = [
            ("README.md", r"C:\Users\me\proj\README.md"),
            ("../x", r"C:\Users\me\x"),
            (r"C:\Windows\", r"C:\Windows"),
            ("/c/Windows/x", r"C:\Windows\x"),
            ("/tmp/f", r"C:\Users\me\AppData\Local\Temp\f"),
            (r"\Windows", r"C:\Windows"),
            ("D:foo", r"D:\foo"),
            (r"C:\", r"C:\"),
        ];
        for (input, want) in cases {
            assert_eq!(resolve(cwd, input, &m), want, "{input}");
        }
        // No Git root known: a rooted path lands on cwd's drive, as Windows reads it.
        assert_eq!(resolve(r"D:\w", "/etc/x", &Mounts::default()), r"D:\etc\x");
    }

    #[test]
    fn within_ignores_case_and_separators() {
        assert!(is_within(r"C:\Users\Me\proj\a", r"c:\users\me\proj"));
        assert!(is_within(r"C:\Users\me\proj", r"C:\Users\me\proj\"));
        assert!(is_within("C:/x/y", r"C:\x"));
        assert!(is_within(r"C:\x", r"C:\"));
        assert!(!is_within(r"C:\Users\me\project2", r"C:\Users\me\proj"));
        assert!(!is_within(r"D:\x", r"C:\"));
    }

    #[test]
    fn command_name_strips_dirs_and_pathext() {
        let ext = ".COM;.EXE;.BAT;.CMD";
        assert_eq!(
            command_name(r"C:\Program Files\Git\usr\bin\ls.exe", ext),
            "ls"
        );
        assert_eq!(command_name("/usr/bin/bash.EXE", ext), "bash");
        assert_eq!(command_name("npm.cmd", ext), "npm");
        assert_eq!(command_name("ls", ext), "ls");
        assert_eq!(command_name("script.py", ext), "script.py");
        assert_eq!(command_name(".exe", ext), ".exe");
    }

    #[test]
    fn mounts_prefer_the_standard_install() {
        let env = |k: &str| match k {
            "EXEPATH" => Some(r"D:\Tools\Git\bin".to_string()),
            "TEMP" => Some(r"C:\T".to_string()),
            _ => None,
        };
        let m = Mounts::from_lookup(env, |_| true);
        assert_eq!(m.root.as_deref(), Some(DEFAULT_GIT_ROOT));
        let m = Mounts::from_lookup(env, |r| r.starts_with('D'));
        assert_eq!(m.root.as_deref(), Some(r"D:\Tools\Git"));
        assert_eq!(m.tmp.as_deref(), Some(r"C:\T"));
        assert_eq!(Mounts::from_lookup(env, |_| false).root, None);
    }
}
