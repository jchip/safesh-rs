# safesh in Rust: Design

Status: approved 2026-11-05.
Repo: `~/dev/safesh-rs`. The TS safesh is at `~/dev/safesh`. TS file paths below refer to that repo.
Related: `architecture.md` (how it is built), `plan.md` (phases and status), `rust-runner-spike-results.md` (P0 evidence), `rust-shell-spike.html` (visual summary), and `bash-architecture-review-2026-06.md` in the TS repo (why the transpiler keeps breaking).

## 1. Goal

Rebuild safesh as one Rust binary. It is a non-interactive bash runner for AI agents. It parses bash, runs commands directly, and checks every command at the point it executes.

The environment stays fairly open. safesh is a guardrail at the exec point, not a jail.

### Why change

Today safesh has two paths. Simple commands pass through to native bash. Everything else is transpiled to TypeScript and run under Deno, which means emulating bash semantics in TS. Most recurring bugs come from that emulation. A real bash engine with a policy hook removes the whole class.

### Non-goals

- No interactive shell. No line editor, completion or prompt.
- No zsh dialect. Agents write bash: only 2 of 15,831 transcript commands used zsh-only syntax.
- No OS sandbox (Seatbelt, Landlock, AppContainer).
- No TypeScript mode, stdlib, JSR library, task runner or MCP server.
- No `external` config or `src/external` port. Argv rules (4.2) replace them.

## 2. Decisions

| Topic | Decision |
|---|---|
| Language | Rust, single binary |
| Dialect | bash, non-interactive |
| Engine | embed `brush-core` (MIT); no fork of brush |
| Control point | exec hook (`ProcessLauncher`) plus a redirect gate (`check_open`) |
| Policy base | `deno_permissions` behind a facade, in Deno's enforcement shape |
| OS sandbox | none |
| Posture | allowlist plus prompt, same as today, with argv rules on top |
| Prompt timing | static pre-check prompts before the run; runtime hook is the backstop |
| Config | JSON only; one-time converter for `config.ts` |
| Dropped | TS mode, stdlib, transpiler, coreutils emulation, vfs, JSR library, task runner, MCP server, `external` |
| Platforms | macOS and Linux together; Windows via WSL2 first, native later |

Numbered decisions D1 to D6 are in section 9.

## 3. Boundary: what safesh covers

safesh sees every command that **our bash engine** runs. That includes commands inside loops, `$( )`, pipelines, subshells, functions, `eval`, `source`, and nested `bash -c` (4.4).

safesh does **not** see what other programs do on their own. Examples are a Python script calling `os.remove`, `make` running its recipe lines with its own `/bin/sh`, `npm run` scripts, and git hooks. This is accepted for a "fairly open" environment. User docs must say it plainly.

One consequence needs attention. Today, file safety for `rm`, `mv`, `cp`, `chmod` and similar tools comes from Deno's `--allow-write`. Those commands skip the command gate because the TS runtime enforced writes. With Deno gone, the exec hook must check their path arguments itself (4.3).

## 4. Policy model

### 4.1 Command allowlist (carried over)

- `permissions.run` defaults to the current 211-entry `SAFE_COMMANDS`.
- Approvals at once, session, project and global scope work as today.
- `allowProjectCommands` keeps running any file under the workspace roots.
- New: the check uses the **resolved program path**, not only the basename (D2). This closes today's gap where `/tmp/x/git` passes because `git` is allowed.

### 4.2 Argv rules (new)

Prefix rules on the expanded argv, with three results: allow, ask, deny. Deny beats ask, and ask beats allow.

```json
"rules": [
  { "match": ["git", "push", "--force*"], "action": "ask" },
  { "match": ["git", "push", "-f"],       "action": "ask" },
  { "match": ["sudo", "**"],              "action": "ask" },
  { "match": ["curl", "**", "|", "sh"],   "action": "ask" }
]
```

Pipeline-shaped rules such as `curl | sh` are matched by the static pre-check. The runtime hook sees one process at a time.

### 4.3 Path arguments for file-changing commands (new, required)

A command-spec table says which argv slots are write or read paths for each known command. The runtime hook checks those paths against `permissions.write` and `denyWrite`.

| Command | Write paths |
|---|---|
| `rm`, `rmdir`, `touch`, `mkdir`, `truncate`, `shred` | all operands |
| `mv`, `cp`, `install`, `ln`, `rsync` | destination, plus sources for `mv` |
| `chmod`, `chown`, `chgrp` | operands after the mode or owner |
| `tee` | all operands |
| `sed -i`, `perl -i` | file operands |
| `dd` | `of=` |
| `tar -x`, `unzip` | `-C` / `-d` target, or cwd |
| `curl -o`, `wget -O` | output file |
| `find -delete`, `git clean` | search root, or cwd |

Rules:

- Paths are made absolute against the runtime cwd. Symlinks are resolved, as today.
- An `ask` result goes through the normal retry flow.
- A known writer with an argv shape we can't parse results in `ask`, not allow.
- The table is mechanical to build. Its tests come from GNU and BSD man pages.

### 4.4 Wrappers

| Wrapper | Handling |
|---|---|
| `env`, `nice`, `nohup`, `time`, `timeout`, `stdbuf` | strip the wrapper and its flags; check the inner argv |
| `xargs` | check `xargs <cmd>` as `<cmd>`; path args are unknown, so writers `ask` |
| `find -exec/-execdir/-ok` | check the `-exec` argv; `{}` counts as a path under the search root |
| `sudo`, `doas` | always `ask` |
| `bash -c`, `sh -c`, `bash script.sh` | re-exec safesh as the child shell, so the payload hits the hook |
| `ssh`, `docker exec`, `kubectl exec` | the command runs elsewhere; check the outer command only |

The nested-shell trick only covers shells started by our engine. A `bash` started by `make` or `python` is outside the boundary (section 3).

### 4.5 Redirects and builtins

- `>`, `>>` and `<>` targets are checked against write rules. `<` targets are checked against read rules.
- `source` and `.` are checked against read rules. `exec` goes through the exec hook.
- `cd` is not gated. It sets the cwd that later checks use.
- brush's optional in-process coreutils builtins stay disabled, so `rm` and friends are always real execs.

### 4.6 Static pre-check

This is a Rust port of `passthrough-analyzer.ts`, built on the brush-parser AST. It extracts command words, redirect targets, cd targets, final cwd, and pipeline-shaped rule matches. Anything not allowed causes a deny-with-retry before the run, as today.

When the analyzer can't resolve something, such as `$x` as a command name, it allows the run and the runtime hook decides. This removes the static-resolution bug class without weakening checks.

## 5. Runtime denial

A command can pass the pre-check and still be denied at runtime. An example is `x=$(pick); $x foo`. When that happens:

1. The exec hook returns an error, and brush reports exit status 126.
2. safesh writes a pending file with the denied argv and cwd.
3. stderr gets the same `[SAFESH] BLOCKED` text and retry command the hook prints today.
4. The script stops. A policy denial forces a `set -e` style abort, so later commands don't run on a broken assumption.

Partial side effects are possible in this case. The pre-check keeps it rare.

## 6. Config, state and migration

### 6.1 Config

- Same file locations, merge order and merge rules as today. Lists merge by union, and deny lists only grow.
- JSON only. TS-only fields are dropped: `imports`, `denoFlags`, `vfs`, `nativeCommands`, `projectTemp`, `alwaysTranspile`, `tasks`, `external`. The loader warns when it sees one.
- New fields: `rules` (4.2), plus optional overrides for the command-spec table.
- Converter: the **last TS release** adds `desh migrate-config`. It imports each `config.ts` and `config.local.ts` with Deno and writes the JSON equivalent next to it. It runs under Deno because `.ts` configs can hold arbitrary code. The Rust binary warns when it finds a `.ts` config with no `.json` beside it.

### 6.2 State files

- Session files keep their format and location: `<project>/.temp/safesh/session-<id>.json`.
- Pending files keep their format: `/tmp/safesh/pending/`, with a 5-minute TTL.
- Scratchpad trust rules port unchanged.
- Worktree detection ports unchanged.
- Only cwd persists between agent calls, through the `&& cd '<final>'` trailer, as today.

### 6.3 Behavior changes

| Change | Status |
|---|---|
| Drop TS mode, library, task runner, MCP, `external` | approved 2026-11-05 |
| JSON-only config plus converter | approved 2026-11-05 |
| Path-arg checks for file-changing commands (4.3) | replaces Deno write enforcement; needed for parity |
| Check resolved program path, not basename (D2) | approved 2026-11-05 |
| Key session grants by hook stdin `session_id`, not env `CLAUDE_SESSION_ID` (D3) | approved 2026-11-05 |
| One choice numbering across all agents (D4) | approved 2026-11-05 |

Any further change that breaks current behavior needs sign-off before it ships.

## 7. Platforms

- **macOS and Linux:** the same code, with no sandbox code to differ. CI runs on both.
- **Windows via WSL2:** this is the Linux build, so it only needs docs and a CI smoke test.
- **Native Windows (later, experimental):**
  - Covers only the Claude Code Bash tool when it runs through Git Bash. The PowerShell tool is not covered.
  - brush on Windows is experimental, and UNC paths are a known gap.
  - Policy needs extra handling:
    - Resolve through PATHEXT and exec the resolved absolute path.
    - Treat `cmd /c`, `powershell -Command`, `wsl.exe` and `bash.exe` as `ask` wrappers.
    - Normalize case and `\\?\` paths before matching.

## 8. Testing strategy

| Suite | Source | Use |
|---|---|---|
| Policy spec | ~650 shell-independent TS tests (policy, config, session, pending, retry) | port as Rust tests; same inputs, same decisions |
| Hook golden files | recorded stdin/stdout JSON per agent | byte-compatible output check |
| Analyzer differential | 15,831 transcript commands | parse-only: diff TS analyzer vs Rust analyzer output (commands, redirects, final cwd) |
| Engine conformance | `tests/bugs` (57), `tests/conformance/differential.test.ts`, field-issue files | run in brush and in bash in a temp dir; diff stdout, stderr, status |
| Hook coverage | new | `rm` in loops, `$( )`, `$x`, `eval`, `bash -c`, `xargs`, `find -exec`, redirects |

The transcript corpus is never executed. It is parsed only.

## 9. Decisions (approved 2026-11-05)

- **D1:** P0 spike approved and done. It passed; see `rust-runner-spike-results.md`.
- **D2:** check the resolved program path, not just the basename.
- **D3:** key session grants by hook stdin `session_id`. Existing `session-default.json` grants stop applying.
- **D4:** one choice numbering, the hook's 1-5, for every agent.
- **D5:** the Rust code lives in its own repo, `~/dev/safesh-rs`, which has no remote yet.
- **D6:** enforcement follows Deno's shape, with `deno_permissions` as the policy base behind a facade. See `architecture.md` section 3.
