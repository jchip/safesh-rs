//! `safesh install <agent>`: add the safesh hook to an agent's settings.
//!
//! An existing TS safesh hook is replaced.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::{fail, self_exe};

const CODEX_BEGIN: &str = "# BEGIN SAFESH CODEX HOOKS";
const CODEX_END: &str = "# END SAFESH CODEX HOOKS";

pub fn install(agent: &str, dry_run: bool) -> u8 {
    let exe = self_exe().to_string_lossy().into_owned();
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    let result = match agent {
        "claude" => claude(&home.join(".claude/settings.json"), &exe, dry_run),
        "codex" => {
            let dir =
                std::env::var_os("CODEX_HOME").map_or_else(|| home.join(".codex"), PathBuf::from);
            codex(&dir.join("config.toml"), &exe, dry_run)
        }
        "agy" => {
            let dir = std::env::var_os("GEMINI_CONFIG_DIR")
                .map_or_else(|| home.join(".gemini/config"), PathBuf::from);
            agy(&dir.join("hooks.json"), &exe, dry_run)
        }
        _ => Err("agent must be claude, codex or agy".into()),
    };
    match result {
        Ok(msg) => {
            println!("{msg}");
            0
        }
        Err(e) => fail(&e),
    }
}

/// Seconds a `PreToolUse` hook may wait for the user; kept under `PRE_TOOL_TIMEOUT`.
const BUDGET_SEC: u32 = 590;
/// Agent-side `PreToolUse` hook timeout in seconds.
const PRE_TOOL_TIMEOUT: u32 = 600;

fn hook_command(exe: &str, agent: &str) -> String {
    let mut c = format!("{} hook {agent}", crate::hook::sh_quote(exe));
    c.push_str(" --budget-sec ");
    c.push_str(&BUDGET_SEC.to_string());
    c
}

fn with_ts_note(msg: String, replaced_ts: bool) -> String {
    if replaced_ts {
        format!("{msg}\nreplaced the TS safesh hook")
    } else {
        msg
    }
}

fn write_file(path: &Path, content: &str, dry_run: bool) -> Result<String, String> {
    if dry_run {
        return Ok(format!("would write {}:\n{content}", path.display()));
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let tmp = path.with_extension("safesh-tmp");
    std::fs::write(&tmp, content).map_err(|e| format!("{}: {e}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(format!("updated {}", path.display()))
}

fn read_json(path: &Path) -> Result<Value, String> {
    match std::fs::read_to_string(path) {
        Ok(t) if t.trim().is_empty() => Ok(json!({})),
        Ok(t) => serde_json::from_str(&t).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

fn is_ts_hook(cmd: &str) -> bool {
    cmd.contains("bash-prehook") || cmd.contains("desh")
}

fn is_our_hook(cmd: &str) -> bool {
    cmd.contains(" hook claude") && !is_ts_hook(cmd)
}

/// Claude Code `settings.json`: a `PreToolUse` entry for Bash.
fn claude(path: &Path, exe: &str, dry_run: bool) -> Result<String, String> {
    let mut settings = read_json(path)?;
    let pre = settings
        .as_object_mut()
        .ok_or("settings.json is not an object")?
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("hooks is not an object")?
        .entry("PreToolUse")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or("hooks.PreToolUse is not an array")?;
    // Drop any TS safesh hook and any earlier safesh-rs entry, then add the new one.
    let mut replaced_ts = false;
    for m in pre.iter_mut() {
        if let Some(hooks) = m["hooks"].as_array_mut() {
            hooks.retain(|h| {
                let Some(c) = h["command"].as_str() else {
                    return true;
                };
                let ts = is_ts_hook(c);
                replaced_ts |= ts;
                !(ts || is_our_hook(c))
            });
        }
    }
    pre.retain(|m| m["hooks"].as_array().is_none_or(|h| !h.is_empty()));
    pre.push(json!({
        "matcher": "Bash",
        "hooks": [{ "type": "command", "command": hook_command(exe, "claude"), "timeout": PRE_TOOL_TIMEOUT }]
    }));
    let text = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())? + "\n";
    write_file(path, &text, dry_run).map(|m| with_ts_note(m, replaced_ts))
}

/// Codex `config.toml`: a marked block, replaced on reinstall.
fn codex(path: &Path, exe: &str, dry_run: bool) -> Result<String, String> {
    let existing = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let cmd = hook_command(exe, "codex");
    // The PermissionRequest hook is quick and takes no budget flag.
    let perm_cmd = cmd.replace(&format!(" --budget-sec {BUDGET_SEC}"), "");
    let block = format!(
        "{CODEX_BEGIN}\n# Managed by safesh install codex. Rerun it to update.\n\
[[hooks.PreToolUse]]\nmatcher = \"^Bash$\"\n\n[[hooks.PreToolUse.hooks]]\ntype = \"command\"\n\
command = {cmd:?}\ntimeout = {PRE_TOOL_TIMEOUT}\nstatusMessage = \"Routing Bash through safesh\"\n\n\
[[hooks.PermissionRequest]]\nmatcher = \"^Bash$\"\n\n[[hooks.PermissionRequest.hooks]]\n\
type = \"command\"\ncommand = {perm_cmd:?}\ntimeout = 5\nstatusMessage = \"Authorizing safesh runner\"\n{CODEX_END}\n"
    );
    let mut replaced_ts = false;
    let content = match (existing.find(CODEX_BEGIN), existing.find(CODEX_END)) {
        (Some(b), Some(e)) if e > b => {
            let old = &existing[b..e];
            replaced_ts = is_ts_hook(old);
            let end = e + CODEX_END.len();
            let rest = existing[end..]
                .strip_prefix('\n')
                .unwrap_or(&existing[end..]);
            format!("{}{block}{rest}", &existing[..b])
        }
        _ if existing.contains("bash-prehook") => {
            return Err("unmarked safesh Codex hooks found; remove them first".into());
        }
        _ => {
            let sep = if existing.is_empty() || existing.ends_with("\n\n") {
                ""
            } else if existing.ends_with('\n') {
                "\n"
            } else {
                "\n\n"
            };
            format!("{existing}{sep}{block}")
        }
    };
    write_file(path, &content, dry_run).map(|m| {
        format!(
            "{}\nRun /hooks in Codex and trust the new hooks.",
            with_ts_note(m, replaced_ts)
        )
    })
}

/// Antigravity `hooks.json`: a `safesh` key.
fn agy(path: &Path, exe: &str, dry_run: bool) -> Result<String, String> {
    let mut cfg = read_json(path)?;
    let obj = cfg.as_object_mut().ok_or("hooks.json is not an object")?;
    let replaced_ts = obj
        .get("safesh")
        .is_some_and(|v| is_ts_hook(&v.to_string()));
    obj.remove("safesh-shadow");
    obj.insert(
        "safesh".into(),
        json!({
            "enabled": true,
            "PreToolUse": [{
                "matcher": "run_command",
                "hooks": [{ "type": "command", "command": hook_command(exe, "agy"), "timeout": PRE_TOOL_TIMEOUT }]
            }]
        }),
    );
    let text = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())? + "\n";
    write_file(path, &text, dry_run).map(|m| with_ts_note(m, replaced_ts))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_replaces_ts_hook() {
        let dir = std::env::temp_dir().join(format!("safesh-install-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("settings.json");
        std::fs::write(
            &path,
            r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"deno run hooks/bash-prehook.ts"}]}]}}"#,
        )
        .unwrap();
        let msg = claude(&path, "/x/safesh", false).unwrap();
        assert!(msg.ends_with("replaced the TS safesh hook"), "{msg}");
        let msg = claude(&path, "/x/safesh", false).unwrap();
        assert!(!msg.contains("TS safesh"), "{msg}");
        let v = read_json(&path).unwrap();
        let pre = v["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre.len(), 1, "{v}");
        assert!(!v.to_string().contains("bash-prehook"), "{v}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn codex_block_replaced_in_place() {
        let dir = std::env::temp_dir().join(format!("safesh-codex-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.toml");
        std::fs::write(&path, "model = \"x\"\n").unwrap();
        codex(&path, "/x/safesh", false).unwrap();
        codex(&path, "/y/safesh", false).unwrap();
        let t = std::fs::read_to_string(&path).unwrap();
        assert_eq!(t.matches(CODEX_BEGIN).count(), 1);
        assert!(t.contains("/y/safesh") && !t.contains("/x/safesh") && t.starts_with("model"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn tmp(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("safesh-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn hook_command_has_budget() {
        assert_eq!(
            hook_command("/x/safesh", "claude"),
            "'/x/safesh' hook claude --budget-sec 590"
        );
        assert!(hook_command("/x/safesh", "agy").ends_with("hook agy --budget-sec 590"));
    }

    #[test]
    fn claude_replaces_old_entry_and_sets_timeout() {
        let dir = tmp("old-claude");
        let path = dir.join("settings.json");
        std::fs::write(
            &path,
            r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"/old/safesh hook claude","timeout":30}]}]}}"#,
        )
        .unwrap();
        claude(&path, "/x/safesh", false).unwrap();
        let v = read_json(&path).unwrap();
        let pre = v["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre.len(), 1, "{v}");
        let h = &pre[0]["hooks"][0];
        assert_eq!(h["command"], "'/x/safesh' hook claude --budget-sec 590");
        assert_eq!(h["timeout"], 600);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn agy_replaces_old_entry_and_sets_timeout() {
        let dir = tmp("old-agy");
        let path = dir.join("hooks.json");
        std::fs::write(
            &path,
            r#"{"safesh":{"enabled":true,"PreToolUse":[{"matcher":"run_command","hooks":[{"type":"command","command":"/old/safesh hook agy","timeout":30}]}]}}"#,
        )
        .unwrap();
        agy(&path, "/x/safesh", false).unwrap();
        let v = read_json(&path).unwrap();
        assert_eq!(v.as_object().unwrap().len(), 1, "{v}");
        let h = &v["safesh"]["PreToolUse"][0]["hooks"][0];
        assert_eq!(h["command"], "'/x/safesh' hook agy --budget-sec 590");
        assert_eq!(h["timeout"], 600);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn codex_replaces_old_block_and_sets_timeouts() {
        let dir = tmp("old-codex");
        let path = dir.join("config.toml");
        let old = format!(
            "model = \"x\"\n\n{CODEX_BEGIN}\n[[hooks.PreToolUse.hooks]]\ncommand = \"/old/safesh hook codex\"\ntimeout = 30\n{CODEX_END}\nafter = 1\n"
        );
        std::fs::write(&path, old).unwrap();
        codex(&path, "/x/safesh", false).unwrap();
        let t = std::fs::read_to_string(&path).unwrap();
        assert_eq!(t.matches(CODEX_BEGIN).count(), 1, "{t}");
        assert!(
            !t.contains("/old/safesh") && !t.contains("timeout = 30"),
            "{t}"
        );
        assert!(t.contains("command = \"'/x/safesh' hook codex --budget-sec 590\"\ntimeout = 600"));
        assert!(t.contains("command = \"'/x/safesh' hook codex\"\ntimeout = 5"));
        assert!(t.starts_with("model") && t.trim_end().ends_with("after = 1"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn agy_replaces_ts_hook_and_drops_shadow_key() {
        let dir = tmp("ts-agy");
        let path = dir.join("hooks.json");
        std::fs::write(
            &path,
            r#"{"safesh":{"PreToolUse":[{"hooks":[{"command":"deno run bash-prehook.ts"}]}]},"safesh-shadow":{}}"#,
        )
        .unwrap();
        let msg = agy(&path, "/x/safesh", false).unwrap();
        assert!(msg.ends_with("replaced the TS safesh hook"), "{msg}");
        let v = read_json(&path).unwrap();
        assert_eq!(v.as_object().unwrap().len(), 1, "{v}");
        assert!(!v.to_string().contains("bash-prehook"), "{v}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn codex_replaces_ts_block() {
        let dir = tmp("ts-codex");
        let path = dir.join("config.toml");
        let old = format!("{CODEX_BEGIN}\ncommand = \"deno run bash-prehook.ts\"\n{CODEX_END}\n");
        std::fs::write(&path, old).unwrap();
        let msg = codex(&path, "/x/safesh", false).unwrap();
        assert!(msg.contains("replaced the TS safesh hook"), "{msg}");
        let t = std::fs::read_to_string(&path).unwrap();
        assert!(
            !t.contains("bash-prehook") && t.contains("/x/safesh"),
            "{t}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
