# safesh in Rust: Architecture

Status: approved 2026-11-05. Type and module names are a starting point for P1 and may shift during the build.
Related: `design.md` (what and why), `plan.md` (phases and status), `rust-runner-spike-results.md` (P0 evidence).

## 1. Overview

safesh has two entry points and one policy core.

- `safesh hook <agent>` runs as the agent's PreToolUse hook. It parses the command, checks it up front, and either denies with a retry hint or rewrites the command to `safesh run`.
- `safesh run --id <hash>` runs the approved script in an embedded brush shell. Every exec and every file open asks the same policy core again.

Both entry points send the same `Effect` values into the same core. So the up-front check and the runtime check can't disagree on a rule.

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
        ├─ check_open       every > >> < and source target path
        └─ builtin gate     exec (through the launcher), cd sets cwd
   runtime deny → exit 126, stderr retry hint, pending file, abort
```

## 2. Request flows

### 2.1 Hook call

1. Read stdin JSON. Detect the agent protocol and pull out the command, cwd, `session_id`, timeout and background flag.
2. Pass control-plane commands through untouched: `safesh run`, `safesh retry`, `safesh retry-path`.
3. Load config: global, project, local, then session grants keyed by `session_id`.
4. Parse with brush-parser. Walk the AST and emit `Effect`s plus the final cwd.
5. Run each effect through the core.
   - All allowed: cache the script by content hash and rewrite the command to `safesh run --id <hash>`. Add `&& cd '<final>'` when the final cwd is known and the run is not in the background.
   - Any ask or deny: write a pending file and return deny JSON with the 1-5 retry menu.
   - Unresolvable parts, such as `$x` as a command name: allow here and leave it to the runtime.
6. Print the agent-specific JSON and exit.

### 2.2 Run

1. Load the cached script and config.
2. Build a brush `Shell` with `PolicyLauncher` and coreutils builtins disabled.
3. Run the script. The launcher checks each exec. `check_open` checks each redirect and `source`.
4. On a runtime deny: write a pending file, print `[SAFESH] BLOCKED` with the retry command to stderr, and abort the run with exit 126.

### 2.3 Retry

`safesh retry --id <id> --choice <1-5>` loads the pending file and applies the grant. The grant goes to once, project config, session file or global config, or the choice denies. Then it runs the cached script through the same path as 2.2. `retry-path` does the same for path grants.

## 3. Enforcement core (Deno shape)

In Deno, every op that touches the outside world makes one call to a shared `PermissionsContainer`. The op asks, the container decides, and a pluggable prompter handles "prompt". Ops hold no policy logic. safesh uses the same shape.

```
 SOURCES (thin adapters)          CORE (pure, no I/O)                          OUTCOME
 brush ProcessLauncher.spawn ─┐
 brush check_open (> < source)├─► Effect ─► Interpret ─► [Request..] ─► Policy ─► Decision ─► Responder
 static pre-check (AST walk) ─┘   Exec|Open   command-spec             Allow/Ask/Deny     hook JSON │ pending+abort │ record
                                              table + wrappers
```

| Stage | What it does |
|---|---|
| **Effect** | Two kinds only: `Exec { program, argv, cwd, env }` and `Open { path, read \| write }`. |
| **Interpret** | Turns an `Exec` into typed requests. `mv a ../b` becomes `Run(/bin/mv)`, `Write(a)` and `Write(../b)`. `xargs rm` becomes `Run(xargs)` and `Run(rm)`. Command knowledge lives only in the command-spec table and the wrapper rules. |
| **Policy** | `check(&Request) -> Decision`. Path, run and env lists go through `deno_permissions`. Argv rules and session grants sit on top. |
| **Responder** | One per mode, like Deno's `PermissionPrompter`. The hook emits deny-with-retry JSON. The runtime writes the pending file, prints `BLOCKED` and aborts. Shadow mode and tests record the decision. |

### 3.1 Core types (sketch)

```rust
enum Effect {
    Exec { program: PathBuf, argv: Vec<OsString>, cwd: PathBuf, env: Env },
    Open { path: PathBuf, cwd: PathBuf, access: Access },   // Read | Write
}

enum Request { Run(PathBuf), Read(PathBuf), Write(PathBuf), Env(String), Argv(Vec<OsString>) }

enum Decision { Allow, Ask(Reason), Deny(Reason) }

trait Responder { fn respond(&self, effect: &Effect, decision: &Decision) -> Outcome; }
```

The core takes its I/O through injected traits: fs metadata for symlink and path resolution, and a grants store for session and pending files. Policy tests run without a shell or a real filesystem.

### 3.2 Rules that keep checks in one place

- The brush glue is two small functions with no policy in them. They convert to an `Effect`, call the core, and map the `Decision` to an `io::Error`.
- No `if cmd == "rm"` in code. Adding a tool means adding a command-spec row.
- Only the `policy` facade module imports `deno_permissions`.
- The static pre-check emits `Effect`s and never decides on its own.

## 4. Reuse of `deno_permissions`

`deno_permissions` (MIT) is Deno's own permission engine, published as a crate. It provides:
- `UnaryPermission<T>` with granted, prompt and denied states
- deny-wins path matching with partial grants
- `which`-based run resolution, which matches D2
- env patterns
- a `PermissionPrompter` trait

The config maps onto it one to one, because TS safesh was built on Deno's flags. `permissions.read`/`denyRead`/`write`/`denyWrite`/`run`/`env` go to `PermissionsOptions`. That gives path-decision parity with today's TS safesh.

Costs and how we handle them:
- Releases come often and break things. Pin an exact version and upgrade on purpose.
- About 11.7k lines, including unused parts (ffi, import, sys). We leave those disabled.
- The prompter is a global static. That's fine for a one-shot CLI process.
- Run permission is name or path only. Argv rules add the argv-level part.

## 5. brush integration

| Point | How |
|---|---|
| Exec | `PolicyLauncher` implements brush's `ProcessLauncher`. It is passed as an `Arc` through the shell builder. `LaunchRequest` gives the resolved program, argv0, expanded args, full env and cwd. |
| Redirects and `source` | brush routes all of them through the private `Shell::open_file`. A 31-line patch adds a default `ProcessLauncher::check_open(path, write)` and calls it at 3 sites. It goes upstream as a PR. Until it lands, we carry it as a vendored patch. |
| Nested shells | The launcher rewrites a `bash` or `sh` exec to the safesh binary. The child is a real process, so PIDs, fds and job control stay native. Cost is about 8 ms per nested shell. |
| Abort | A runtime deny must stop the whole run, including later pipeline stages. The spike used `process::exit(126)`. P3 replaces it with a clean abort path. |
| Builtins | brush's in-process coreutils builtins stay off, so file-changing tools are always real execs. |

brush is pinned to an exact commit or version. Known gap: `wait <pid>` and `wait $!` are unimplemented and are the first upstream fix.

## 6. Workspace layout

| Crate | Job | Depends on |
|---|---|---|
| `safesh-policy` | config load and merge, the `policy` facade over `deno_permissions`, argv rules, command specs, session and pending files | (leaf) |
| `safesh-analyze` | static pre-check on the brush-parser AST, emitting `Effect`s | policy, brush-parser |
| `safesh-exec` | brush-core embedding, `PolicyLauncher`, `check_open` glue, nested shells, abort | policy, brush-core |
| `safesh-hooks` | agent protocols: Claude/Gemini, Codex, agy; byte-compatible output | policy, analyze |
| `safesh-cli` | the `safesh` binary and subcommands | all |

Today only `safesh-policy` and `safesh-cli` exist. The others are added in their phase.

Planned modules in `safesh-policy`:

| Module | Contents |
|---|---|
| `effect` | `Effect`, `Request`, `Decision`, `Responder` |
| `policy` | facade over `deno_permissions`; the only importer of it |
| `config` | JSON load, merge rules, dropped-field warnings |
| `rules` | argv rule matching |
| `spec` | command-spec table and wrapper rules (`Interpret`) |
| `paths` | absolute, normalize, soft canonicalize, scratchpad trust, project root |
| `state` | session files, pending files, atomic writes |

## 7. CLI surface

| Command | Purpose |
|---|---|
| `safesh hook claude\|gemini\|codex\|agy` | PreToolUse entry point (Codex also PermissionRequest) |
| `safesh run --id <hash>` | run an approved, cached script in brush |
| `safesh retry --id <id> --choice <1-5>` | approve a pending command, then run it |
| `safesh retry-path --id <id> --choice <...>` | approve a pending path, then run it |
| `safesh check '<cmd>'` | show the pre-check result; for debugging |
| `safesh install claude\|codex\|agy` | install hooks (ports today's installers) |

`desh` becomes an alias of `safesh` for one release, so pending retry hints in flight keep working.

## 8. Files on disk

| File | Path | Written by |
|---|---|---|
| Global config | `$XDG_CONFIG_HOME/safesh/config.json` | user, retry choice 5 |
| Project config | `<project>/.config/safesh/config.json` | user |
| Local config | `<project>/.config/safesh/config.local.json` | user, retry choice 2 |
| Session grants | `<project>/.temp/safesh/session-<id>.json` | retry choice 3 |
| Pending | `/tmp/safesh/pending/pending-<id>.json`, `pending-path-<id>.json` | hook and run on deny; 5-minute TTL |
| Script cache | `/tmp/safesh/scripts/` | hook on allow |
| Codex approvals | `/tmp/safesh/approvals/` | Codex hook; mode 0600 |

## 9. Chosen crates

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
