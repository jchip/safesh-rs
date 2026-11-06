# safesh in Rust: Execution Plan

Status as of 2026-11-05: P0 done. P1 exit check met; three small items left.
Related: `design.md` (what and why), `architecture.md` (how it is built).

Each phase has an exit check. Tasks marked **(mech)** are mechanical and go to lower-model subagents. The TS repo stays in use until cutover in P6.

## Status

| Phase | What | Status |
|---|---|---|
| P0 | Hands-on spike: brush, policy launcher, redirect gate | done, passed |
| P1 | Workspace and policy core | exit check met; small items left |
| P2 | Static pre-check | in progress |
| P3 | Exec engine | not started |
| P4 | Agent adapters | not started |
| P5 | Shadow mode | not started |
| P6 | Cutover | not started; needs migration sign-off |
| P7 | Windows | not started |

## P0. Hands-on spike (done)

- Build brush on macOS and Linux.
- Write a PolicyLauncher prototype. Confirm brush accepts a stateful launcher instance.
- Find the redirect open point. Decide between a brush extension and a small upstream patch.
- Prove nested `bash -c` runs under the policy. The answer was to re-exec safesh as the child shell.
- Measure cold start of `safesh run`. Target under 30 ms.
- **(mech)** Run `tests/bugs`, the differential suite and the field issues through brush and bash, then diff.

Exit check: the hook catches the coverage cases, and brush matches bash on most of the corpus. If redirect gating needs a fork, stop and decide.

Result: passed. 97.6% stdout and status match, 9.4 ms cold start, redirect gate needs a 31-line patch. See `rust-runner-spike-results.md`.

Follow-ups from P0, assigned to later phases:
- Upstream PR for `check_open` (P3).
- Upstream fix for `wait <pid>` (P3).
- Clean abort-on-denial path in place of `process::exit` (P3).
- Full bash flag parsing for nested shells (P3).
- Corpus rerun against bash 5 on Linux and macOS in CI (P3, needs brush in CI).

## P1. Workspace and policy core (exit check met)

- **(mech)** Set up the cargo workspace, CI for macOS and Linux, clippy and fmt. **Done.**
- Write the `policy` facade over `deno_permissions`, with `Effect`, `Request` and `Decision`. **Done.** The `Responder` trait moves to P3/P4, where its callers live.
- Write the config loader with today's merge rules. **Done.**
- Write the session and pending file I/O. **Done.**
- **(mech)** Port the paths, project-root, scratchpad, config, session and pending tests. **Done**, 200 tests.
- **(mech)** Port the policy tests (command, path, effective permissions, session merge) and audit row coverage. **Done**, 273 tests in all. Every spec row has a test, a SKIP, or an `external` reason.

Exit check: the ported tests pass with the same decisions as TS.

### P1 next steps

Scratch inputs are in `.temp/`, which is gitignored:

| File | Use |
|---|---|
| `.temp/p1-test-spec.md` | 330 test rows extracted from the TS tests, with SKIP and CHANGED-BY-DESIGN marks. Port from this. |
| `.temp/port-inventory.md` | Config fields, merge rules, default lists, state files, hook protocols |
| `.temp/crate-survey.md` | Crate picks with versions (also in `architecture.md` section 9) |
| `.temp/brush-spike/` | brush clone with the `check_open` patch, builds, corpus report |

1. Make the scratchpad tests hermetic. Two of them touch the real `/tmp/claude-<uid>`, as the TS tests did.
2. Decide on cwd `/`. The facade adds the base cwd to default read, so cwd `/` makes every path readable (deny lists still win). TS missed this case only through an `isPathWithin` quirk.
3. The bash 5 corpus rerun moves to P3, since it needs brush built in CI.

Rows pv-*, cv-5, cv-7 and pd-13 to pd-15 stay skipped. They cover `external`, which is out of scope.

## P2. Static pre-check

- Port `passthrough-analyzer.ts` onto the brush-parser AST, emitting `Effect`s.
- **(mech)** Build the transcript-corpus differential harness (parse-only).

Exit check: the differential shows no unexplained diffs, and `tests/bugs` analyzer cases pass.

## P3. Exec engine

- Embed brush-core with PolicyLauncher, `check_open` and the builtin gate.
- Add wrapper handling and the runtime denial flow, including a clean abort.
- Full bash flag parsing for nested shells.
- **(mech)** Build the command-spec table and its tests from man pages.
- Write the argv rules engine.
- Open the upstream PRs for `check_open` and `wait <pid>`.
- **(mech)** Commit the engine corpus and run it against bash 5 on Linux and macOS in CI.

Exit check: the hook-coverage suite and the engine conformance corpus pass. `xargs rm` and `find -exec rm` are caught.

## P4. Agent adapters

- Write the Claude/Gemini, Codex and agy protocols, plus retry and retry-path.
- **(mech)** Record golden stdin/stdout files from the TS hooks. Then port the installers.

Exit check: the golden files match byte for byte.

## P5. Shadow mode

- The TS hook stays in charge. It also calls `safesh hook --shadow`, which logs the Rust decision without acting on it.
- Run this for daily use. Review the diffs weekly.

Exit check: two weeks with no unexplained decision diffs.

## P6. Cutover (needs sign-off on the migration plan)

- Ship the last TS release with `desh migrate-config`.
- Switch the hook installers to `safesh`. Keep the `desh` alias for one release.
- Move the TS sources to an archive branch. Move the TS-era notes to `notes/archive`.

## P7. Windows

- WSL2: add docs and a CI smoke test.
- Native: build the experimental path from `design.md` section 7.
