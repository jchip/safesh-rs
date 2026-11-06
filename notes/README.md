# safesh-rs notes

## Documents

| File | What it is |
|---|---|
| `rust-runner-design.md` | **Design and execution plan**: architecture, policy model, decisions, phases P0-P7 |
| `rust-runner-spike-results.md` | P0 spike results: hook coverage, corpus compatibility, latency |
| `rust-shell-spike.html` | Early research summary (OS sandbox sections are superseded by the design doc) |

The TS repo is at `~/dev/safesh`. Port sources live there under `src/core`, `src/hooks` and `hooks/`.

## Status (2026-11-05)

- P0 spike: done and passed.
- P1 policy core: in progress. The workspace and CI are scaffolded. The policy code is not written yet.

## P1 handoff

Scratch inputs are in `.temp/`, which is gitignored:

| File | Use |
|---|---|
| `.temp/p1-test-spec.md` | 330 test rows extracted from the TS tests, with SKIP and CHANGED-BY-DESIGN marks. Port from this. |
| `.temp/port-inventory.md` | Config fields, merge rules, default lists, state files, hook protocols |
| `.temp/crate-survey.md` | Crate picks with versions (also in the design doc, section 4.3) |
| `.temp/brush-spike/` | brush clone with the `check_open` patch, builds, corpus report |

Next steps:
1. Add `deno_permissions` (pinned) behind `safesh-policy::policy`. Map config `permissions.*` to `PermissionsOptions`.
2. Write the config loader (JSON, today's merge rules) and the session and pending file I/O.
3. Port the spec rows for permissions, config, session, pending, project-root and scratchpad.
4. `src/external` and the `external` config field are out of scope. Skip spec rows pv-*, cv-5, cv-7 and pd-13 to pd-15.
