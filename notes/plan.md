# safesh in Rust: Execution Plan

Status as of 2026-11-05: alpha complete. P0 to P4 are done, and the P5 shadow-mode tooling is in place. Running shadow mode for two weeks is next.
Related: `design.md` (what and why), `architecture.md` (how it is built).

Each phase has an exit check. Tasks marked **(mech)** are mechanical and go to lower-model subagents. The TS repo stays in use until cutover in P6.

## Status

| Phase | What | Status |
|---|---|---|
| P0 | Hands-on spike: brush, policy launcher, redirect gate | done, passed |
| P1 | Workspace and policy core | done |
| P2 | Static pre-check | done |
| P3 | Exec engine | done; upstream PRs and CI corpus left |
| P4 | Agent adapters | done |
| P5 | Shadow mode | tooling done; two-week run not started |
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

### Open items after alpha

Scratch inputs and harnesses are in `.temp/`, which is gitignored: `p1-test-spec.md`, `port-inventory.md`, `p2-corpus/` (transcript differential), `alpha-conformance/` (engine corpus), `smoke/smoke.sh`.

1. Run shadow mode for two weeks and review `shadow.jsonl` diffs weekly (P5).
2. Open the upstream brush PRs: `check_open` on `ProcessLauncher`, and `wait <pid>`.
3. Commit the engine corpus and run it against bash 5 on Linux and macOS in CI.
4. Decide on cwd `/`. The facade adds the base cwd to default read, so cwd `/` makes every path readable (deny lists still win). TS missed this case only through an `isPathWithin` quirk.
5. Make the scratchpad tests hermetic. Two of them touch the real `/tmp/claude-<uid>`, as the TS tests did.
6. Keep backgrounded builtins alive until they finish, or document the limit.

Rows pv-*, cv-5, cv-7 and pd-13 to pd-15 stay skipped. They cover `external`, which is out of scope.

## P2. Static pre-check (done)

- Replace `passthrough-analyzer.ts` and the prehook extraction with `safesh-analyze` on the brush-parser AST. **Done**, 145 tests. Walk rules are in `design.md` 4.6.
- **(mech)** Build the transcript-corpus differential harness (parse-only). **Done**: `.temp/p2-corpus/`.

Exit check: the differential shows no unexplained diffs. Result on 15,633 agent commands (11,209 TS-eligible compared):
- final cwd matches on all 11,209.
- Commands: Rust is a superset. It lists builtins and the bodies of `$( )`, `<( )` and here-docs, which TS skipped. No command TS found is missing in Rust, apart from one function call, which is excluded by design.
- Redirects match apart from path spelling: Rust keeps them relative, with the cwd stored separately.
- Pipeline-shaped rules (`curl ** | sh`) are checked against every pipeline the analyzer reports.

## P3. Exec engine

- Embed brush-core with PolicyLauncher, `check_open` and the builtin gate. **Done** (`safesh-exec`, vendored patched brush-core).
- Add wrapper handling and the runtime denial flow, including a clean abort. **Done.** A nested shell's denial signals its parent, so the whole run stops.
- Full bash flag parsing for nested shells. **Done.**
- **(mech)** Build the command-spec table and its tests from man pages. **Done**, 136 tests.
- Write the argv rules engine. **Done.**
- Open the upstream PRs for `check_open` and `wait <pid>`. Not started.
- **(mech)** Commit the engine corpus and run it against bash 5 on Linux and macOS in CI. Not started.

Exit check: the hook-coverage suite and the engine conformance corpus pass. `xargs rm` and `find -exec rm` are caught. Result:
- 40 end-to-end tests cover the hook-coverage cases, retry flows, nested shells, wrappers, rules and install.
- Conformance: 482 of 497 corpus snippets match bash on stdout and status (spike baseline 485). The differences are the 7 known `wait <pid>` gaps, old spike failures, three harness artifacts, and `echo hi &` losing its output (below).

Known alpha gaps:
- A backgrounded builtin (`echo hi &`) runs as an in-process brush task and dies when safesh exits, so its output can be lost.
- `wait <pid>` is unimplemented in brush.

## P4. Agent adapters (done)

- Write the Claude/Gemini, Codex and agy protocols, plus retry and retry-path. **Done** (`safesh-hooks`, `safesh-cli`).
- **(mech)** Record golden stdin/stdout files from the TS hooks. Then port the installers. **Done.** `safesh install claude|codex|agy`, with `-shadow` forms.

Exit check: the golden files match. They match structurally, with documented substitutions: `safesh run` for `desh -q`, the rewrite reason text, normalized pending ids, and the dropped TypeScript HINT.

## P5. Shadow mode

- The TS hook stays in charge. A second hook entry runs `safesh hook <agent> --shadow`, which logs the Rust decision to `<state>/shadow.jsonl` without acting on it. Install it with `safesh install claude-shadow`. **Tooling done.**
- Run this for daily use. Review the diffs weekly.

Exit check: two weeks with no unexplained decision diffs.

## P6. Cutover (needs sign-off on the migration plan)

- Ship the last TS release with `desh migrate-config`.
- Switch the hook installers to `safesh`. Keep the `desh` alias for one release.
- Move the TS sources to an archive branch. Move the TS-era notes to `notes/archive`.

## P7. Windows

- WSL2: add docs and a CI smoke test.
- Native: build the experimental path from `design.md` section 7.
