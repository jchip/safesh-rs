//! `safesh install <agent>`: add the safesh hook to an agent's settings.
//!
//! An existing TS safesh hook is left in place. Installing next to it is only allowed in shadow
//! mode, since switching over needs sign-off.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::{fail, self_exe};

const CODEX_BEGIN: &str = "# BEGIN SAFESH CODEX HOOKS";
const CODEX_END: &str = "# END SAFESH CODEX HOOKS";

pub fn install(agent: &str, dry_run: bool) -> u8 {
    let (agent, shadow) = match agent.strip_suffix("-shadow") {
        Some(a) => (a, true),
        None => (agent, false),
    };
    let exe = self_exe().to_string_lossy().into_owned();
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    let result = match agent {
        "claude" => claude(&home.join(".claude/settings.json"), &exe, shadow, dry_run),
        "codex" => {
            let dir =
                std::env::var_os("CODEX_HOME").map_or_else(|| home.join(".codex"), PathBuf::from);
            codex(&dir.join("config.toml"), &exe, shadow, dry_run)
        }
        "agy" => {
            let dir = std::env::var_os("GEMINI_CONFIG_DIR")
                .map_or_else(|| home.join(".gemini/config"), PathBuf::from);
            agy(&dir.join("hooks.json"), &exe, shadow, dry_run)
        }
        _ => Err("agent must be claude, codex or agy (add -shadow for shadow mode)".into()),
    };
    match result {
        Ok(msg) => {
            println!("{msg}");
            0
        }
        Err(e) => fail(&e),
    }
}

fn hook_command(exe: &str, agent: &str, shadow: bool) -> String {
    let mut c = format!("{} hook {agent}", crate::hook::sh_quote(exe));
    if shadow {
        c.push_str(" --shadow");
    }
    c
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
fn claude(path: &Path, exe: &str, shadow: bool, dry_run: bool) -> Result<String, String> {
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
    let commands: Vec<String> = pre
        .iter()
        .flat_map(|m| m["hooks"].as_array().cloned().unwrap_or_default())
        .filter_map(|h| h["command"].as_str().map(str::to_string))
        .collect();
    if !shadow && commands.iter().any(|c| is_ts_hook(c)) {
        return Err(
            "the TS safesh hook is installed; use `safesh install claude-shadow`, or remove the TS hook first".into(),
        );
    }
    // Drop any earlier safesh-rs entry, then add the new one.
    for m in pre.iter_mut() {
        if let Some(hooks) = m["hooks"].as_array_mut() {
            hooks.retain(|h| !h["command"].as_str().is_some_and(is_our_hook));
        }
    }
    pre.retain(|m| m["hooks"].as_array().is_none_or(|h| !h.is_empty()));
    pre.push(json!({
        "matcher": "Bash",
        "hooks": [{ "type": "command", "command": hook_command(exe, "claude", shadow) }]
    }));
    let text = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())? + "\n";
    write_file(path, &text, dry_run)
}

/// Codex `config.toml`: a marked block, replaced on reinstall.
fn codex(path: &Path, exe: &str, shadow: bool, dry_run: bool) -> Result<String, String> {
    let existing = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let cmd = hook_command(exe, "codex", shadow);
    let block = format!(
        "{CODEX_BEGIN}\n# Managed by safesh install codex. Rerun it to update.\n\
[[hooks.PreToolUse]]\nmatcher = \"^Bash$\"\n\n[[hooks.PreToolUse.hooks]]\ntype = \"command\"\n\
command = {cmd:?}\ntimeout = 30\nstatusMessage = \"Routing Bash through safesh\"\n\n\
[[hooks.PermissionRequest]]\nmatcher = \"^Bash$\"\n\n[[hooks.PermissionRequest.hooks]]\n\
type = \"command\"\ncommand = {cmd:?}\ntimeout = 5\nstatusMessage = \"Authorizing safesh runner\"\n{CODEX_END}\n"
    );
    let content = match (existing.find(CODEX_BEGIN), existing.find(CODEX_END)) {
        (Some(b), Some(e)) if e > b => {
            let old = &existing[b..e];
            if !shadow && is_ts_hook(old) {
                return Err(
                    "the TS safesh Codex hooks are installed; use `safesh install codex-shadow`"
                        .into(),
                );
            }
            if shadow && is_ts_hook(old) {
                return Err("Codex allows one managed safesh block; shadow mode next to the TS hooks is not supported for Codex".into());
            }
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
    write_file(path, &content, dry_run)
        .map(|m| format!("{m}\nRun /hooks in Codex and trust the new hooks."))
}

/// Antigravity `hooks.json`: a `safesh` key, or `safesh-shadow` in shadow mode.
fn agy(path: &Path, exe: &str, shadow: bool, dry_run: bool) -> Result<String, String> {
    let mut cfg = read_json(path)?;
    let obj = cfg.as_object_mut().ok_or("hooks.json is not an object")?;
    let key = if shadow { "safesh-shadow" } else { "safesh" };
    if !shadow
        && obj
            .get("safesh")
            .is_some_and(|v| is_ts_hook(&v.to_string()))
    {
        return Err("the TS safesh agy hook is installed; use `safesh install agy-shadow`".into());
    }
    obj.insert(
        key.into(),
        json!({
            "enabled": true,
            "PreToolUse": [{
                "matcher": "run_command",
                "hooks": [{ "type": "command", "command": hook_command(exe, "agy", shadow), "timeout": 30 }]
            }]
        }),
    );
    let text = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())? + "\n";
    write_file(path, &text, dry_run)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_refuses_next_to_ts_hook_unless_shadow() {
        let dir = std::env::temp_dir().join(format!("safesh-install-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("settings.json");
        std::fs::write(
            &path,
            r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"deno run hooks/bash-prehook.ts"}]}]}}"#,
        )
        .unwrap();
        assert!(claude(&path, "/x/safesh", false, false).is_err());
        claude(&path, "/x/safesh", true, false).unwrap();
        claude(&path, "/x/safesh", true, false).unwrap();
        let v = read_json(&path).unwrap();
        let pre = v["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre.len(), 2, "{v}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn codex_block_replaced_in_place() {
        let dir = std::env::temp_dir().join(format!("safesh-codex-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.toml");
        std::fs::write(&path, "model = \"x\"\n").unwrap();
        codex(&path, "/x/safesh", false, false).unwrap();
        codex(&path, "/y/safesh", false, false).unwrap();
        let t = std::fs::read_to_string(&path).unwrap();
        assert_eq!(t.matches(CODEX_BEGIN).count(), 1);
        assert!(t.contains("/y/safesh") && !t.contains("/x/safesh") && t.starts_with("model"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
