# safesh in Rust: Design and Execution Plan

Status: draft for review, 2026-11-05.
Repo: `~/dev/safesh-rs` (this repo). The TS safesh is at `~/dev/safesh`; TS file paths below refer to that repo.
Related: `rust-shell-spike.html` (research summary), and `bash-architecture-review-2026-06.md` in the TS repo (why the transpiler keeps breaking).
Raw research: `.temp/research-*.md`, `.temp/port-inventory.md`.

## 1. Goal

Rebuild safesh as one Rust binary. It is a non-interactive bash runner for AI agents. It parses bash, runs commands directly, and checks every command at the point it executes.

The environment stays fairly open. safesh is a guardrail at the exec point, not a jail.

### Non-goals

- No interactive shell. No line editor, completion or prompt.
- No zsh dialect. Agents write bash: only 2 of 15,831 transcript commands used zsh-only syntax.
- No OS sandbox (Seatbelt, Landlock, AppContainer).
- No TypeScript mode, stdlib, JSR library, task runner or MCP server.

## 2. Decisions

| Topic | Decision |
|---|---|
| Language | Rust, single binary |
| Dialect | bash, non-interactive |
| Engine | embed `brush-core` (MIT); no fork of brush |
| Control point | exec hook (`ProcessLauncher`) plus a redirect gate |
| OS sandbox | none |
| Posture | allowlist plus prompt, same as today, with argv rules on top |
| Prompt timing | static pre-check prompts before the run; runtime hook is the backstop |
| Config | JSON only; one-time converter for `config.ts` |
| Dropped | TS mode, stdlib, transpiler, coreutils emulation, vfs, JSR library, task runner, MCP server |
| Platforms | macOS and Linux together; Windows via WSL2 first, native later |

## 3. Boundary: what safesh covers

safesh sees every command that **our bash engine** runs. That includes commands inside loops, `$( )`, pipelines, subshells, functions, `eval`, `source`, and nested `bash -c` (see 5.4).

safesh does **not** see what other programs do on their own. Examples are a Python script calling `os.remove`, `make` running its recipe lines with its own `/bin/sh`, `npm run` scripts, and git hooks. This is accepted for a "fairly open" environment. The docs must say it plainly.

One consequence needs attention. Today, file safety for `rm`, `mv`, `cp`, `chmod` and similar tools comes from Deno's `--allow-write`. Those commands skip the command gate because the TS runtime enforced writes. With Deno gone, the exec hook must check their path arguments itself (5.3).

## 4. Architecture

```
agent (Claude Code / Gemini / Codex / agy)
   │ PreToolUse JSON
   ▼
safesh hook <agent> ── static pre-check (brush-parser AST)
   │   allowed  → rewrite command to:  safesh run --id <hash> && cd '<final cwd>'
   │   blocked  → deny with retry hint, write pending file
   ▼
safesh run --id <hash>
   └─ brush-core Shell
        ├─ PolicyLauncher   every external exec: argv, cwd, env
        ├─ redirect gate    every > >> < target path
        └─ builtin gate     source, exec, cd
   runtime deny → exit 126, stderr retry hint, pending file
```

### 4.1 Enforcement architecture (modelled on Deno)

In Deno, every op that touches the outside world makes one call to a shared `PermissionsContainer`. The op asks, the container decides, and a pluggable prompter handles "prompt". Ops hold no policy logic. safesh uses the same shape:

```
 SOURCES (thin adapters)          CORE (pure, no I/O)                          OUTCOME
 brush ProcessLauncher.spawn ─┐
 brush check_open (> < source)├─► Effect ─► Interpret ─► [Request..] ─► Policy ─► Decision ─► Responder
 static pre-check (AST walk) ─┘   Exec|Open   command-spec             Allow/Ask/Deny     hook JSON │ pending+abort │ record
                                              table + wrappers
```

| Stage | What it does |
|---|---|
| **Effect** | Two kinds only: `Exec { program, argv, cwd, env }` and `Open { path, read \| write }`. The runtime and the static pre-check emit the same effects into the same core, so they can't disagree. |
| **Interpret** | Turns an `Exec` into typed requests. `mv a ../b` becomes `Run(/bin/mv)`, `Write(a)` and `Write(../b)`. `xargs rm` becomes `Run(xargs)` and `Run(rm)`. Command knowledge lives only in a data table of command specs (5.3, 5.4). |
| **Policy** | `check(&Request) -> Decision`. Path, run and env lists use `deno_permissions` (4.2). Argv rules (5.2) and session grants sit on top. |
| **Responder** | One per mode, like Deno's `PermissionPrompter`. The hook pre-check emits deny-with-retry JSON. The runtime writes the pending file, prints `BLOCKED` and aborts. Shadow mode and tests record the decision. |

Rules that keep checks from spreading through the code:
- The brush glue is two small functions with no policy in them. They convert to an `Effect`, call the core, and map the `Decision` to an `io::Error`.
- No `if cmd == "rm"` in code. Adding a tool means adding a spec-table row.
- The core does no I/O except through injected traits (fs metadata, grants store), so policy tests run without a shell.
- Only the `policy` facade module imports `deno_permissions`.

### 4.2 Reuse of `deno_permissions`

`deno_permissions` (MIT) is Deno's own permission engine, published as a crate. It provides:
- `UnaryPermission<T>` with granted, prompt and denied states
- deny-wins path matching with partial grants
- `which`-based run resolution, which matches D2
- env patterns
- a `PermissionPrompter` trait

The config already maps onto it one to one, because TS safesh was built on Deno's flags: `permissions.read`/`denyRead`/`write`/`denyWrite`/`run`/`env` go to `PermissionsOptions`. That gives path-decision parity with today's TS safesh.

Costs and how we handle them:
- Releases come often and break things. Pin an exact version and upgrade on purpose.
- About 11.7k lines, including unused parts (ffi, import, sys). We leave those disabled.
- The prompter is a global static. That's fine for a one-shot CLI process.
- Run permission is name or path only. Our argv rules add the argv-level part.

### 4.3 Chosen crates

| Need | Crate |
|---|---|
| shell engine, parser | `brush-core`, `brush-builtins`, `brush-parser` (pin exact; `brush-parser` is standalone for P2) |
| permission engine | `deno_permissions` (pin exact) |
| config vars `${CWD}` `${HOME}` `~` | `shellexpand` |
| absolute and normalized paths, soft canonicalize | `path-absolutize`, `soft-canonicalize` (wrapped in one function) |
| env masks and globs | `globset` |
| XDG config dir (also on macOS) | `etcetera` |
| PATH resolution | `which` |
| uid and owner checks | `rustix` |
| atomic state writes | `tempfile` (persist) |
| config parsing and schema | `serde`, `serde_json`, `serde_path_to_error`, `schemars` |
| fs errors with paths | `fs-err` |
| errors | `thiserror` |
| CLI, logging, ids | `clap`, `tracing`, `tracing-subscriber`, `sha2` |
| argv operand parsing | `lexopt`; uutils `uu_rm`/`uu_cp`/`uu_mv` `uu_app()` as test oracles only |
| shell quoting for `cd '<dir>'` | `shlex` |
| tests | `insta`, `assert_cmd`, `predicates` |

We write three things ourselves: the argv rule engine (Codex's `execpolicy` isn't published standalone), the command-spec table, and the hook protocol structs.

### 4.4 Cargo workspace (`crates/`)

| Crate | Job |
|---|---|
| `safesh-policy` | config load and merge, the `policy` facade over `deno_permissions`, argv rules, command specs, session and pending files |
| `safesh-analyze` | static pre-check on the brush-parser AST, emitting `Effect`s |
| `safesh-exec` | brush-core embedding, PolicyLauncher and `check_open` glue, nested shells |
| `safesh-hooks` | agent protocols: Claude/Gemini, Codex, agy; byte-compatible output |
| `safesh-cli` | the `safesh` binary and subcommands |

The TS repo stays in use until cutover (phase P6).

### CLI surface

| Command | Purpose |
|---|---|
| `safesh hook claude\|gemini\|codex\|agy` | PreToolUse entry point (Codex also PermissionRequest) |
| `safesh run --id <hash>` | run an approved, cached script in brush |
| `safesh retry --id <id> --choice <1-5>` | approve a pending command, then run it |
| `safesh retry-path --id <id> --choice <...>` | approve a pending path, then run it |
| `safesh check '<cmd>'` | show the pre-check result; for debugging |
| `safesh install claude\|codex\|agy` | install hooks (ports today's installers) |

`desh` becomes an alias of `safesh` for one release, so pending retry hints in flight keep working.

## 5. Policy model

### 5.1 Command allowlist (carried over)

- `permissions.run` defaults to the current 211-entry `SAFE_COMMANDS`.
- Approvals at once, session, project and global scope work as today.
- `allowProjectCommands` keeps running any file under the workspace roots.
- New: the check uses the **resolved program path**, not only the basename. This closes today's gap where `/tmp/x/git` passes because `git` is allowed. See open decision D2.

### 5.2 Argv rules (new)

Prefix rules on the expanded argv, with three results: allow, ask, deny. Deny beats ask, and ask beats allow.

```json
"rules": [
  { "match": ["git", "push", "--force*"], "action": "ask" },
  { "match": ["git", "push", "-f"],       "action": "ask" },
  { "match": ["sudo", "**"],              "action": "ask" },
  { "match": ["curl", "**", "|", "sh"],   "action": "ask" }
]
```

Pipeline-shaped rules such as `curl | sh` are matched by the static pre-check, since the runtime hook sees one process at a time. The existing `external` subcommand and flag config is converted into these rules.

### 5.3 Path arguments for file-changing commands (new, required)

A table says which argv slots are write or read paths for each known command. The runtime hook checks those paths against `permissions.write` and `denyWrite`.

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
- This table is mechanical to build. Its tests come from GNU and BSD man pages.

### 5.4 Wrappers

| Wrapper | Handling |
|---|---|
| `env`, `nice`, `nohup`, `time`, `timeout`, `stdbuf` | strip the wrapper and its flags; check the inner argv |
| `xargs` | check `xargs <cmd>` as `<cmd>`; path args are unknown, so writers `ask` |
| `find -exec/-execdir/-ok` | check the `-exec` argv; `{}` counts as a path under the search root |
| `sudo`, `doas` | always `ask` |
| `bash -c`, `sh -c`, `bash script.sh` | run inside our brush engine as a nested shell, so the payload hits the hook |
| `ssh`, `docker exec`, `kubectl exec` | the command runs elsewhere; check the outer command only |

The nested-shell trick only covers shells started by our engine. A `bash` started by `make` or `python` is outside the boundary (section 3).

### 5.5 Redirects and builtins

- `>`, `>>` and `<>` targets are checked against write rules. `<` targets are checked against read rules.
- `source` and `.` are checked against read rules. `exec` goes through `ProcessLauncher::exec`.
- `cd` is not gated. It sets the cwd that later checks use.
- brush's optional in-process coreutils builtins stay disabled, so `rm` and friends are always real execs.

### 5.6 Static pre-check

This is a Rust port of `passthrough-analyzer.ts`, built on the brush-parser AST. It extracts command words, redirect targets, cd targets, final cwd, and pipeline-shaped rule matches. Anything not allowed causes a deny-with-retry before the run, as today.

When the analyzer can't resolve something, such as `$x` as a command name, it allows the run and the runtime hook decides. This removes the static-resolution bug class (SSH-673, 728, 746) without weakening checks.

## 6. Runtime denial

A command can pass the pre-check and still be denied at runtime. An example is `x=$(pick); $x foo`. When that happens:

1. PolicyLauncher returns an error, and brush reports exit status 126.
2. safesh writes a pending file with the denied argv and cwd.
3. stderr gets the same `[SAFESH] BLOCKED` text and retry command the hook prints today.
4. The script stops. `set -e` style abort is forced on a policy denial, so later commands don't run on a broken assumption.

Partial side effects are possible in this case. The pre-check keeps it rare.

## 7. Config, state and migration

### 7.1 Config

- Same file locations, merge order and merge rules as today (union for lists, deny lists only grow).
- JSON only. Fields that are TS-only get dropped: `imports`, `denoFlags`, `vfs`, `nativeCommands`, `projectTemp`, `alwaysTranspile`, `tasks`.
- New fields: `rules` (5.2), plus optional overrides for the arg-spec table.
- Converter: the **last TS release** adds `desh migrate-config`. It imports each `config.ts` and `config.local.ts` with Deno, then writes the JSON equivalent next to it. It runs under Deno because `.ts` configs can hold arbitrary code. The Rust binary warns when it finds a `.ts` config with no `.json` beside it.

### 7.2 State files

- Session files keep their format and location: `<project>/.temp/safesh/session-<id>.json`.
- Pending files keep their format: `/tmp/safesh/pending/`, with a 5-minute TTL.
- Scratchpad trust rules port unchanged.
- Worktree detection ports unchanged.
- Only cwd persists between agent calls, through the `&& cd '<final>'` trailer, as today.

### 7.3 Behavior changes that need sign-off

| Change | Status |
|---|---|
| Drop TS mode, library, task runner, MCP | approved 2026-11-05 |
| JSON-only config plus converter | approved 2026-11-05 |
| Path-arg checks for file-changing commands (5.3) | replaces Deno write enforcement; needed for parity |
| Check resolved program path, not basename (D2) | approved 2026-11-05 |
| Key session grants by hook stdin `session_id` rather than env `CLAUDE_SESSION_ID` (D3) | approved 2026-11-05 |
| One choice numbering across all agents (D4) | approved 2026-11-05 |

## 8. Platforms

- **macOS and Linux:** the same code, with no sandbox code to differ. CI runs on both.
- **Windows via WSL2:** this is the Linux build, so it only needs docs and a CI smoke test.
- **Native Windows (later, experimental):**
  - Covers only the Claude Code Bash tool when it runs through Git Bash. The PowerShell tool is not covered.
  - brush on Windows is experimental, and UNC paths are a known gap.
  - Policy needs extra handling:
    - Resolve through PATHEXT and exec the resolved absolute path.
    - Treat `cmd /c`, `powershell -Command`, `wsl.exe` and `bash.exe` as `ask` wrappers.
    - Normalize case and `\\?\` paths before matching.

## 9. Testing

| Suite | Source | Use |
|---|---|---|
| Policy spec | ~650 shell-independent TS tests (policy, config, session, pending, retry) | port as Rust tests; same inputs, same decisions |
| Hook golden files | recorded stdin/stdout JSON per agent | byte-compatible output check |
| Analyzer differential | 15,831 transcript commands | parse-only: diff TS analyzer vs Rust analyzer output (commands, redirects, final cwd) |
| Engine conformance | `tests/bugs` (57), `tests/conformance/differential.test.ts`, field-issue files | run in brush and in bash in a temp dir; diff stdout, stderr, status |
| Hook coverage | new | `rm` in loops, `$( )`, `$x`, `eval`, `bash -c`, `xargs`, `find -exec`, redirects |

The transcript corpus is never executed. It is parsed only.

## 10. Execution plan

Each phase has an exit check. Tasks marked **(mech)** are mechanical and go to lower-model subagents.

### P0. Hands-on spike (needs your OK: clones and builds brush)

- Build brush on macOS and Linux.
- Write a PolicyLauncher prototype. Confirm brush accepts a stateful launcher instance.
- Find the redirect open point. Decide between a brush extension and a small upstream patch.
- Prove nested `bash -c` can run in-process.
- Measure cold start of `safesh run`. Target under 30 ms.
- **(mech)** Run `tests/bugs`, the differential suite and the field issues through brush and bash, then diff.

Exit check: the hook catches the cases in section 9, and brush matches bash on most of the corpus. If redirect gating needs a fork, stop and decide.

### P1. Workspace and policy core

- **(mech)** Set up the cargo workspace, CI for macOS and Linux, clippy and fmt. Done.
- Write the `policy` facade over `deno_permissions`, with `Effect`, `Request`, `Decision` and the `Responder` trait (4.1).
- Write the config loader with today's merge rules.
- Write the session and pending file I/O.
- **(mech)** Port the policy, config, session and pending tests.

Exit check: the ported tests pass with the same decisions as TS.

### P2. Static pre-check

- Port `passthrough-analyzer.ts` onto the brush-parser AST.
- **(mech)** Build the transcript-corpus differential harness (parse-only).

Exit check: the differential shows no unexplained diffs, and `tests/bugs` analyzer cases pass.

### P3. Exec engine

- Embed brush-core with PolicyLauncher, the redirect gate and the builtin gate.
- Add wrapper handling (5.4) and the runtime denial flow (6).
- **(mech)** Build the arg-spec table (5.3) and its tests from man pages.
- Write the argv rules engine. **(mech)** Convert `external` config entries into rules.

Exit check: the hook-coverage suite and the engine conformance corpus pass.

### P4. Agent adapters

- Write the Claude/Gemini, Codex and agy protocols, plus retry and retry-path.
- **(mech)** Record golden stdin/stdout files from the TS hooks. Then port the installers.

Exit check: the golden files match byte for byte.

### P5. Shadow mode

- The TS hook stays in charge. It also calls `safesh hook --shadow`, which logs the Rust decision without acting on it.
- Run this for daily use. Review the diffs weekly.

Exit check: two weeks with no unexplained decision diffs.

### P6. Cutover (needs sign-off on the migration plan)

- Ship the last TS release with `desh migrate-config`.
- Switch the hook installers to `safesh`. Keep the `desh` alias for one release.
- Move the TS sources to an archive branch. Move the TS-era notes to `notes/archive`.

### P7. Windows

- WSL2: add docs and a CI smoke test.
- Native: build the experimental path from section 8.

## 11. Decisions (approved 2026-11-05)

- **D1:** P0 spike approved and done. It passed; see `rust-runner-spike-results.md`.
- **D2:** check the resolved program path, not just the basename.
- **D3:** key session grants by hook stdin `session_id`. Existing `session-default.json` grants stop applying.
- **D4:** one choice numbering, the hook's 1-5, for every agent.
- **D5:** the Rust code lives in its own repo, `~/dev/safesh-rs`, which has no remote yet.
- **D6 (proposed, awaiting review):** enforcement follows Deno's shape (4.1), with `deno_permissions` as the policy base behind a facade (4.2).
