# safesh-rs notes

safesh-rs is the Rust rewrite of safesh. It is a non-interactive bash runner for AI agents. It runs commands in an embedded brush shell and checks every exec and file open against a policy.

The TS repo is at `~/dev/safesh`. Port sources live there under `src/core`, `src/hooks` and `hooks/`.

## Documents

| File | What it is |
|---|---|
| `design.md` | **Design**: goal, boundary, policy model, runtime denial, config and migration, decisions D1-D6 |
| `architecture.md` | **Architecture**: request flows, Deno-style enforcement core, brush integration, crates and modules, files on disk |
| `plan.md` | **Execution plan**: phases P0-P7 with status, exit checks and P1 next steps |
| `interactive-approval.md` | **Draft**: asking the user directly (Claude Code ask, tmux popup, GUI dialog) with text fallback |
| `approval-spike-results.md` | A1 spike: which approval channels work from a hook process |
| `windows-spike-results.md` | P7: native Windows under Git Bash, the seven gaps and how each was fixed, and what is still open |
| `approval-a2-plan.md` | A2 plan: channel trait, tmux and GUI channels, `approve-ui`, lock and answer files |
| `approval-a3-plan.md` | A3 plan: the hook asks the user at the pre-check, once-grants, hook time budget |
| `rust-runner-spike-results.md` | P0 spike results: hook coverage, corpus compatibility, latency |
| `rust-shell-spike.html` | Visual summary of the final design (published artifact) |
| `archive/` | Superseded docs. `rust-runner-design.md` was split into the three docs above. |

## Status (2026-11-08)

Alpha complete. The `safesh` binary runs the hook, the pre-check, the embedded bash engine with runtime gates, retry, and install for Claude, Codex and agy. 717 tests pass on macOS. Shadow mode was removed on 2026-11-12. A field test with the Rust hook in charge started on 2026-11-06. Native Windows under Git Bash is in as experimental (P7): CI builds and tests it on `windows-latest`, and its known gaps are listed in the top-level README and `windows-spike-results.md`. Open items are listed in `plan.md`.
