# safesh (Rust)

A non-interactive bash runner for AI coding agents. It parses bash with [brush](https://github.com/reubeno/brush), runs commands directly, and checks every command and redirect against a permission policy before it happens.

This is the Rust rewrite of [safesh](https://github.com/jchip/safesh) (Deno/TypeScript). Status: **alpha**. See `notes/README.md` for the design and status.

## Build

```sh
cargo build --release
cargo test --workspace
```

The binary is `target/release/safesh`. Put it on your `PATH`; the retry hints it prints call `safesh` by name.

## How it works

1. The agent's PreToolUse hook calls `safesh hook <agent>`. It parses the command and checks every command, redirect and `source` target it can see.
2. Allowed: the hook rewrites the command to `safesh run --id <hash>`, plus `&& cd '<dir>'` when the script changes directory.
3. Not allowed: the hook denies with a retry menu. The agent asks you, then runs `safesh retry --id=<id> --choice=<n>`.
4. `safesh run` executes the script in an embedded bash engine. Every exec and file open is checked again at runtime. A denial stops the whole run with status 126 and prints the same retry menu.

safesh only sees what its own shell runs. A Python script deleting files, or `make` running its own `/bin/sh`, is outside its reach.

## Platforms

- **macOS and Linux:** the main targets. CI builds and tests macOS arm64, Linux x86_64 and Linux arm64.
- **Windows: experimental.** Native Windows under Git Bash, the shell Claude Code's Bash tool uses there. CI builds and tests it on `windows-latest`. Git Bash paths (`/c/x`, `/tmp`, `~`) are mapped the way `cygpath` maps them, nested shells are gated, the state dirs are owner-only, scratchpad scripts are trusted, and approvals use a desktop dialog. Known gaps:
  - Arguments to native Windows programs are not converted. Git Bash turns `/c/x` into `C:\x` for programs like `git.exe`; safesh does not, so policy checks `C:\x` while the program reads `C:\c\x`. Use Windows paths (`C:/x`) in arguments to native programs.
  - `#!` scripts do not start, because Windows has no shebang support. Run them as `bash script.sh`.
  - A command run by its Git Bash path (`/usr/bin/ls`, `/tmp/x/tool.exe`) is not mapped at runtime and is blocked.
  - `$PATH` inside a script is the native `;`-separated list, not Git Bash's `:` form, so `PATH=/x/bin:$PATH` does not put `/x/bin` first. Use `PATH="C:/x/bin;$PATH"`.
  - Stale approval-queue tickets are not cleaned up.
  - Not yet tried in a real Claude Code session on Windows.

  Details: `notes/windows-spike-results.md`.

## Install

```sh
safesh install claude          # Claude Code: ~/.claude/settings.json
safesh install codex           # Codex: $CODEX_HOME/config.toml (then /hooks to trust)
safesh install agy             # Antigravity: ~/.gemini/config/hooks.json
```

Add `--dry-run` to print the change without writing it. The installer replaces an existing TS safesh hook.

## Commands

| Command | Purpose |
|---|---|
| `safesh hook claude\|gemini\|codex\|agy` | PreToolUse entry point |
| `safesh run --id <hash>` | run an approved, cached script |
| `safesh retry --id <id> --choice <1-5>` | approve a pending command, then run it |
| `safesh retry-path --id <id> --choice <r\|w\|rw><1-3>[d]` | approve a pending path, then run it |
| `safesh check '<script>'` | show what the pre-check decides |
| `safesh install <agent>` | install the hook |
| `safesh status` | list hooks and runs in progress, with what each is doing now |
| `safesh log [--id P] [--session P] [--date D] [-n N] [-f]` | show recent events, or follow them |

## Debugging

When a command seems stuck, run `safesh status`. A hook may be waiting on an approval prompt nobody sees, or a run may be on a child exec. Then run `safesh log --id <id>` for one script, or `safesh log -f` to follow. Set `SAFESH_DEBUG=1` to also print each event to stderr as it happens.

```
PID    KIND  AGE    ID                SESSION   NOW
48213  hook  1m35s  3f9a1c0b7d2e4a68  a1b2c3d4  waiting for the user via tmux (1m35s)
48377  run   12s    3f9a1c0b7d2e4a68  a1b2c3d4  exec cargo test (12s)
```

Events go to `/tmp/safesh/log/<date>.jsonl` (JSONL, readable with jq). Live status is in `/tmp/safesh/active/<pid>.json`. Logs older than 7 days are removed. `SAFESH_STATE_DIR` replaces `/tmp/safesh`.

## Config

JSON, same files and merge rules as TS safesh:
- `~/.config/safesh/config.json` (or `$XDG_CONFIG_HOME/safesh/config.json`)
- `<project>/.config/safesh/config.json`
- `<project>/.config/safesh/config.local.json`

`config.ts` files are not read. Convert them with `desh migrate-config` from the last TS release.

New in the Rust version: argv `rules`, prefix patterns with `allow`, `ask` or `deny`:

```json
{ "rules": [ { "match": ["git", "push", "--force*"], "action": "ask" } ] }
```

State lives in `/tmp/safesh` (override with `SAFESH_STATE_DIR`).
