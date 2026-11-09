# Windows Spike Results

Date: 2026-11-08. Branch: `spike/windows`, based on `main` at `bb3f89b`.
Box: Legion (`jc-legion9`), Windows 11 Pro build 26300, Rust 1.94.1 MSVC, Git for Windows 2.48.1.
Shell under test: Git Bash, the shell Claude Code's Bash tool uses on Windows.

## Verdict

Native Windows is reachable. The workspace builds for `x86_64-pc-windows-msvc` after small shims, and the brush engine runs scripts. In the first pass it was not usable: Git Bash path translation was missing, so the runtime gate blocked almost every external command, and file paths resolved to the wrong place. Items 1 and 2 in section 3 fix that; see section 5. Under Git Bash, common scripts now run and are gated the same way as on macOS.

WSL2 was not tested. The WSL Rust is 1.88, and the workspace needs 1.89 (`File::try_lock`).

## 1. Build

`main` fails on Windows in three crates. Every failure is a Unix-only API, and none is in brush.

| Site | Unix API | Spike shim |
|---|---|---|
| `safesh-policy/src/host.rs` | `rustix::process::getuid` | uid `0` |
| `safesh-policy/src/scratchpad.rs` | `MetadataExt::uid` | `owns_claude_dir` returns false, so scratchpad trust is off |
| `safesh-hooks/src/approval.rs`, `safesh-cli/src/approval/{files,once}.rs` | `.mode(0o600/0o700)`, `Permissions::from_mode` | `safesh_policy::compat::fs`: `.mode()` is a no-op |
| `safesh-cli/src/approval/files.rs` | `test_kill_process` for queue-ticket liveness | `compat::pid_alive` returns true for any pid > 0 |
| `safesh-cli/src/approval/mod.rs` | exec bit in `find_bin` | any file counts |
| `safesh-exec/src/lib.rs` | SIGUSR1 to parent safesh pids | none at first; item 3 now terminates them |

The vendored brush-core, with the `check_open` patch, compiled with no changes. The shims are about 50 lines plus `compat.rs`. macOS still builds, clippy is clean, and all 695 tests pass.

Tests: the 20 lib and bin unit tests pass on Windows. Eight integration-test files use `std::os::unix` directly (symlinks, modes, uid) and do not compile there. Most of the real coverage sits in those files.

## 2. Behavior (Git Bash, `target/release/safesh.exe`, no user config)

| Case | Result |
|---|---|
| `echo`, `for` loop, `cd crates && pwd`, `bash -c "echo nested ok"` | ran correctly through `hook claude` then `run` |
| `cmd //c dir`, `powershell -Command ...`, `notepad.exe` | pre-check `ask` |
| `ls`, `cat`, `wc`, `head` at runtime | **blocked**: `[SAFESH] BLOCKED: C:\Program Files\Git\usr\bin\ls.exe`, rc 126 |
| `echo x > /c/Windows/f` | denied, but the path shown is `\\?\C:\c\Windows\f`: the MSYS path was not translated |
| `rm -rf ~` | pre-check `allow` (macOS: `ask`). `~` stays `Unknown`, because `HOME=/c/Users/joel1` is not absolute to Windows. The runtime gate is meant to catch it, but `rm.exe` is already blocked, so this was not tested |
| hook output | well-formed. The rewrite is `'C:\...\safesh.exe' run --id ...`, which Git Bash runs as is |
| cold start | about 43 ms per `safesh check`, against 9.4 ms on macOS |

## 3. Gaps, in order

1. **Exe matching.** *(Fixed, see section 5.)* `is_resolved_allowed_name` (`policy.rs:355`) compares the basename `ls.exe` with the rule `ls`. Strip the PATHEXT extension and compare case-insensitively. This is the item in `design.md` section 7.
2. **MSYS paths.** *(Fixed, see section 5.)* Git Bash hands safesh `/c/...`, `/tmp` (which is `%LOCALAPPDATA%\Temp`), `/usr/bin` (which is `C:\Program Files\Git\usr\bin`) and `HOME=/c/Users/...`. Policy needs one translation step, `cygpath` rules or the mount table, before `normalize_lexical`. Without it, `~` and every absolute path are wrong. This is the biggest piece.
3. **Nested denial.** *(Fixed, see section 6.)* A denied `bash -c` exits 126, and the outer script keeps going. Replace SIGUSR1 with `OpenProcess` plus `TerminateProcess(pid, 126)` on `SAFESH_PARENT_PIDS`.
4. **State-dir protection.** With no mode bits, approval and once-grant files inherit the parent's ACL. The deny-write list still guards them from the agent's shell, but other local users are not kept out. That needs an owner-only DACL.
5. **Scratchpad trust.** Claude Code's Windows scratchpad layout and ownership check are unknown. Trust is off for now, which fails closed.
6. **Approval channels.** tmux and GUI are Unix-only. Windows needs its own channel, such as a `MessageBox` or a Windows Terminal pane.
7. **Tests.** Port the integration tests behind `cfg(unix)`, or give them Windows equivalents, and add a `windows-latest` CI job.

## 4. Not tried

- A real Claude Code session on Windows with the hook installed. `safesh install claude` wasn't run, so Legion's Claude settings are untouched.
- WSL2: needs Rust 1.89 or later in the Ubuntu-24.04 distro. The WSL `rustup` binary is missing, and both toolchains are 1.88.
- ARM64 Windows, UNC paths, and the PowerShell tool.

## 5. Items 1 and 2 fixed

Policy and engine read every path through one translator, `safesh_policy::winpath`. It is pure string code, tested on every platform, and `paths` calls it only on Windows.

- **Translation:** `/c/x` becomes `C:\x`, `/tmp` becomes `%TEMP%`, `/` and `/bin` become the Git install, matching `cygpath`. `/dev/*` keeps its spelling, so device checks still match. The Git root is `C:\Program Files\Git` when it holds `usr\bin\msys-2.0.dll`, and only after that `EXEPATH` or `SHELL`, which the agent's shell can set.
- **Windows path rules:** drive and UNC prefixes, both separators, `\\?\` dropped from canonical paths, and case-insensitive containment. `normalize_lexical`, `resolve_lexical` and `is_path_within` send Windows paths here.
- **Engine:** a new brush hook, `ProcessLauncher::host_path`, maps a path inside `Shell::absolute_path`. Redirects, `source`, `cd` and `test` go through it, so brush opens the file the policy checked. `check_open` now gets the absolute path that `open_file` opens. The patch is `vendor/brush-host-path.patch`, applied on top of `brush-check-open.patch`.
- **Command names:** `paths::command_name` drops the dir and a `PATHEXT` extension. It is used by D2 PATH matching, argv rules, command specs, the analyzer and nested-shell rerouting. deno's `which` now gets `PATHEXT`. Paths with `\` count as paths.
- **Host env:** `HOME` falls back to `USERPROFILE`, `TEMP` stands in for `TMPDIR`, and the state dir defaults to `%TEMP%\safesh`.

Found along the way, and fixed:
- **Nested shells escaped the policy.** Rerouting matched `bash`, not `bash.exe`, so `bash -c "..."` ran the real Git Bash unchecked. It is now rerouted, and `bash -c "notepad.exe"` is blocked.
- **`rm -rf ~` was allowed.** `HOME` (`C:\Users\me`) was read as a relative path and joined under the project. It now asks.
- `rsync`/`scp` took `C:\x` for host `C`, so the path skipped local checks. On Windows a drive path is now local.
- The analyzer's `cd` and device checks used `Path::is_absolute`, which is false for `/c/x` and `/dev/null` on Windows.

Verified on Legion in Git Bash, through `hook claude` then `run`:

| Case | Result |
|---|---|
| `ls`, `cat \| wc`, `git status`, `git log` | run |
| `echo > /tmp/f && cat /tmp/f` | file in `%TEMP%`, not `C:\tmp` |
| `... > /dev/null` | works |
| `cd /c/Users && pwd` | `C:\Users`, and the `cd` rewrite is right |
| `rm -rf ~` | `ask` on `C:\Users\joel1` |
| `d=$(echo /c/Windows); echo x > "$d/f"` and `rm -f "$d/f"` | blocked at runtime as `C:\Windows\f`, rc 126, no file written |
| rule `git push` deny vs `g=git.exe; $g push` | denied by the rule. `git.EXE status` matches an `ask` rule |
| `bash -c "notepad.exe"` | blocked inside the nested shell |

macOS: all 701 tests pass and clippy is clean. Windows: unit tests and clippy are clean.

Still open:
- **Arguments to native programs aren't converted.** Git Bash rewrites `/c/x` in arguments for native exes such as `git.exe` and `cargo.exe`, and brush does not. Policy checks `C:\x`, while a native program sees `/c/x` and reads it as `C:\c\x`. MSYS programs such as coreutils `rm` and `ls` read it the way policy does.
- A path command spelled `/usr/bin/ls` isn't mapped. brush resolves it outside `absolute_path`.
- Globs over Git Bash paths (`/c/Users/*`) were not tried.
- **Allow rules bypass resolution, on every platform.** When an argv `allow` rule matches by basename, `check_exec` skips `check_run`. So `/any/dir/git status` passes a `git status` allow rule. Confirmed on macOS. Windows extension stripping widens the match a little, so `git.cmd` matches too.
- Items 4 to 7 above.

## 6. Item 3 fixed

A denied nested shell now stops every safesh above it on Windows too. `stop_parents` in `safesh-exec` reads `SAFESH_PARENT_PIDS` and, for each pid outermost first, calls `OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE)`, `TerminateProcess(pid, 126)`, then `WaitForSingleObject` with a 2 s bound. The wait matters: `TerminateProcess` returns before the target is gone, and a parent must not see its child exit while a safesh above it still runs. Unix is unchanged: SIGUSR1, and the parent's handler exits 126.

This is the only `unsafe` in the workspace. `safesh-exec` copies the workspace lint table with `unsafe_code = "deny"` instead of `forbid`, and the one `cfg(windows)` function has `#[allow(unsafe_code)]`. `windows-sys` 0.61 (already in the lock) is a Windows-only dependency.

Verified: macOS tests pass (707), clippy is clean on macOS and for `aarch64-pc-windows-msvc`.

Verified on Legion in Git Bash, through `hook claude` then `run`, release build of `4c71290`. The denied command is spelled `n=notepad; $n.exe`, so the outer safesh can't see it statically and the denial comes from the nested shell. The baseline is the build before this fix.

| Case | Fixed | Baseline |
|---|---|---|
| `bash -c 'echo nested-start; n=notepad; $n.exe; echo nested-after'; echo after` | rc 126, `nested-start` | rc 0, `nested-start`, `after` |
| `sh s.sh; echo after` (`s.sh` runs the denied command) | rc 126, `s-start` | rc 0, `s-start`, `after` |
| `bash outer.sh; echo after`, where `outer.sh` runs the denying `bash -c` and then `echo o-after` (two levels) | rc 126, `o-start`, `i-start` | rc 0, `o-start`, `i-start`, `o-after`, `after` |
| `bash -c "echo nested-ok"; echo after` (control) | rc 0, `nested-ok`, `after` | same |

In no case did notepad start.
