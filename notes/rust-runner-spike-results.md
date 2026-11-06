# P0 Spike Results: brush as the safesh engine

Date: 2026-11-05. Code: `spikes/brush-policy/`.
brush commit `b2f7782` (brush-core 0.5.0). Built with Rust stable 1.95.

## Verdict

P0 passes, and we can go ahead with P1. brush matches bash on safesh's own corpus. A policy launcher catches every in-shell case. Startup is fast.

## 1. Hook coverage

The policy launcher is a separate crate with no brush fork, about 280 lines including arg parsing. Writable roots were set to a scratch dir.

| Case | Result |
|---|---|
| `rm` in a `for` loop, in `$( )`, in a pipeline `while read`, in a function | blocked, rc 126 |
| `x=rm; $x f1` | blocked |
| `eval "rm f1"` | blocked |
| `exec rm f1` | blocked |
| `bash -c "rm f1"`, `sh s.sh` | blocked: nested shells re-exec the policy binary |
| `echo hi > /outside/x`, `&> /outside/y` | blocked by `check_open` |
| `touch /outside/t`, `cd sub && mv ../f1 ../../outside/` | blocked by path-arg check, after `..` and symlink resolution |
| `git push --force` | blocked (stand-in for ask) |
| writes inside the roots | allowed |
| abort off: `rm f1; echo after` | rm denied with rc 126, the script continued |
| `xargs rm`, `find -exec rm` | **not caught.** P3 wrapper unwrapping handles these. |
| `make` recipe `rm`, `python3 os.remove` | **not caught.** This is outside the boundary by design. |

On a policy denial the spike exits the whole run with 126. That stops later commands and pipeline stages. P3 needs a cleaner abort than `process::exit`, but the behavior is right.

## 2. Answers to the P0 questions

- **Stateful launcher:** yes. `process_launcher` is a builder field with a generated setter, so an `Arc<Policy>` instance passes straight in.
- **Redirect gate:** brush has no hook for this. Every redirect and `source` goes through one private function, `Shell::open_file`. A 31-line patch adds a default `ProcessLauncher::check_open(path, write)` and calls it at 3 sites (`brush-check-open.patch`). This is small enough for an upstream PR. Fallback: carry the patch as a vendored fork.
- **Nested shells:** a launcher that rewrites `program` from `bash`/`sh` to our own binary works. The child is a real process, so PIDs, fds and job control stay native. The cost is about 8 ms per nested shell.
- **LaunchRequest:** it holds the resolved program, argv0, expanded args, full env and cwd. That's everything the policy needs.

## 3. Compatibility on safesh's corpus

The corpus is 520 snippets taken from `tests/bugs`, the differential suite and the field-issue files. Each ran in a temp dir under brush and `/bin/bash` 3.2.57.

| Result | Count |
|---|---|
| run / skipped | 497 / 23 |
| same stdout and status | 485 (97.6%) |
| also same stderr | 454 (91.3%) |
| real brush gaps | 7 (1.4%), all `wait <pid>` / `wait $!` |

- Error wording differs in 31 cases. That is cosmetic.
- `wait` with PIDs is unimplemented in brush. This is the first upstream fix to take on.
- Caveat: there is no bash 5 on this machine. Rerun against bash 5 in P1 CI.
- Report: `.temp/brush-spike/corpus-report.md`.

## 4. Latency

| Command | Mean | p90 |
|---|---|---|
| `bash -c true` | 5.0 ms | 5.7 ms |
| `brush -c true` | 6.6 ms | 8.2 ms |
| `brush-policy -c true` | 9.4 ms | 12.3 ms |
| `brush-policy` pipeline (`ls \| grep \| wc`) | 12.5 ms | 14.6 ms |
| `brush-policy -c 'bash -c true'` (nested) | 17.3 ms | 20.9 ms |
| `deno eval 0` (desh floor today) | 29.2 ms | 31.8 ms |

The binary is 8.3 MB. It starts about 3 times faster than Deno, which desh needs before doing any work.

## 5. New items for the plan

- Upstream PR: `check_open` on `ProcessLauncher` (or its own extension trait), following brush's contribution policy.
- Upstream fix: `wait` with PIDs.
- P3: a clean abort-on-denial path to replace `process::exit`.
- P3: full bash flag parsing for nested shells. The spike rejects options like `-l`, `-i` and `--norc`.
- P1 CI: run the corpus against bash 5 on Linux and macOS.
