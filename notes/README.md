# safesh-rs notes

safesh-rs is the Rust rewrite of safesh. It is a non-interactive bash runner for AI agents. It runs commands in an embedded brush shell and checks every exec and file open against a policy.

The TS repo is at `~/dev/safesh`. Port sources live there under `src/core`, `src/hooks` and `hooks/`.

## Documents

| File | What it is |
|---|---|
| `design.md` | **Design**: goal, boundary, policy model, runtime denial, config and migration, decisions D1-D6 |
| `architecture.md` | **Architecture**: request flows, Deno-style enforcement core, brush integration, crates and modules, files on disk |
| `plan.md` | **Execution plan**: phases P0-P7 with status, exit checks and P1 next steps |
| `rust-runner-spike-results.md` | P0 spike results: hook coverage, corpus compatibility, latency |
| `rust-shell-spike.html` | Visual summary of the final design (published artifact) |
| `archive/` | Superseded docs. `rust-runner-design.md` was split into the three docs above. |

## Status (2026-11-05)

- P0 spike: done and passed.
- P1 policy core: exit check met. `safesh-policy` has config, paths, project root, scratchpad, session and pending state, and the `deno_permissions` facade, with 273 tests. See `plan.md` for the few items left.
