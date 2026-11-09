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
| `safesh-policy/src/scratchpad.rs` | `MetadataExt::uid` | `owns_claude_dir` returned false at first; item 5 checks the dir owner's SID |
| `safesh-hooks/src/approval.rs`, `safesh-cli/src/approval/{files,once}.rs` | `.mode(0o600/0o700)`, `Permissions::from_mode` | `safesh_policy::compat::fs`: `.mode()` is a no-op; item 4 adds an owner-only DACL on the dirs |
| `safesh-cli/src/approval/files.rs` | `test_kill_process` for queue-ticket liveness | `compat::pid_alive` returns true for any pid > 0 |
| `safesh-cli/src/approval/mod.rs` | exec bit in `find_bin` | any file counts; on Windows the only bin dir is the system PowerShell dir (item 6) |
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
4. **State-dir protection.** *(Fixed, see section 7.)* With no mode bits, approval and once-grant files inherit the parent's ACL. The deny-write list still guards them from the agent's shell, but other local users are not kept out. That needs an owner-only DACL.
5. **Scratchpad trust.** *(Fixed, see section 8.)* Claude Code's Windows scratchpad layout and ownership check are unknown. Trust is off for now, which fails closed.
6. **Approval channels.** *(Fixed, see section 9.)* tmux and GUI are Unix-only. Windows needs its own channel, such as a `MessageBox` or a Windows Terminal pane.
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
- A path command spelled `/usr/bin/ls` isn't mapped. brush resolves it outside `absolute_path`. It hits `/tmp/...` too: `cp x /tmp/claude/.../new.exe && /tmp/claude/.../new.exe` passes the pre-check, then is blocked at runtime as `C:/tmp/claude/...`.
- **`#!` scripts don't start.** brush launches programs directly, and Windows has no shebang support, so `./run.sh` or `$PAD/run.sh` fails with `failed to execute command` (rc 126) after the gate allows it. Git Bash itself runs them through MSYS. Running scripts needs brush to read the `#!` line and launch the interpreter.
- Globs over Git Bash paths (`/c/Users/*`) were not tried.
- **Allow rules bypass resolution, on every platform.** When an argv `allow` rule matches by basename, `check_exec` skips `check_run`. So `/any/dir/git status` passes a `git status` allow rule. Confirmed on macOS. Windows extension stripping widens the match a little, so `git.cmd` matches too.
- Item 7 above.

## 6. Item 3 fixed

A denied nested shell now stops every safesh above it on Windows too. `stop_parents` in `safesh-exec` reads `SAFESH_PARENT_PIDS` and, for each pid outermost first, calls `OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE)`, `TerminateProcess(pid, 126)`, then `WaitForSingleObject` with a 2 s bound. The wait matters: `TerminateProcess` returns before the target is gone, and a parent must not see its child exit while a safesh above it still runs. Unix is unchanged: SIGUSR1, and the parent's handler exits 126.

The Win32 calls are `unsafe`. Item 4 moved them, with its own, into `safesh_policy::compat` (`terminate_and_wait`); see section 7.

Verified: macOS tests pass (707), clippy is clean on macOS and for `aarch64-pc-windows-msvc`.

Verified on Legion in Git Bash, through `hook claude` then `run`, release build of `4c71290`. The denied command is spelled `n=notepad; $n.exe`, so the outer safesh can't see it statically and the denial comes from the nested shell. The baseline is the build before this fix.

| Case | Fixed | Baseline |
|---|---|---|
| `bash -c 'echo nested-start; n=notepad; $n.exe; echo nested-after'; echo after` | rc 126, `nested-start` | rc 0, `nested-start`, `after` |
| `sh s.sh; echo after` (`s.sh` runs the denied command) | rc 126, `s-start` | rc 0, `s-start`, `after` |
| `bash outer.sh; echo after`, where `outer.sh` runs the denying `bash -c` and then `echo o-after` (two levels) | rc 126, `o-start`, `i-start` | rc 0, `o-start`, `i-start`, `o-after`, `after` |
| `bash -c "echo nested-ok"; echo after` (control) | rc 0, `nested-ok`, `after` | same |

In no case did notepad start.

## 7. Item 4 fixed

The three private dirs, `approvals`, `approvals-ui` and `once`, are now owner-only on Windows. These are the dirs Unix keeps at 0700. `compat::restrict_dir` replaces the per-site `set_permissions(0o700)`. On Unix it still sets 0700. On Windows it sets a protected DACL (nothing inherited from above) with one full-control ACE for the process user's SID, flagged object- and container-inherit. Files created inside need no call of their own: they inherit that ACE. `SetNamedSecurityInfoW` also gives files already in the dir the new inherited ACE. `.mode()` stays a no-op. `wait_turn` now restricts `approvals-ui` before creating `queue` inside it, because only the dirs passed to `restrict_dir` get the DACL.

The ACE names the token user, not `OWNER RIGHTS`. For an elevated admin, the default owner of new objects is `BUILTIN\Administrators`, so an owner ACE would let in every admin.

Pending and session files live outside these dirs and get default permissions, the same as on Unix. The default state dir, `%TEMP%\safesh`, is under the user profile and already private. The DACL matters when `SAFESH_STATE_DIR` points somewhere shared.

**Unsafe code.** All Win32 FFI now lives in one module, `safesh_policy::compat::win`: `restrict_dir` and the item 3 `terminate_and_wait`. It is the workspace's only `unsafe`. `safesh-policy` copies the workspace lint table with `unsafe_code = "deny"` instead of `forbid`, and the module has `#[allow(unsafe_code)]`. `safesh-exec` is back on the workspace lints. `windows-sys` 0.61, already in the lock, is a Windows-only dependency of `safesh-policy`.

Verified on Legion:
- A `cfg(windows)` unit test restricts a dir that already holds a file, then adds a file. It checks with `icacls` that the dir has exactly one non-inherited `(OI)(CI)(F)` ACE for the user, and that both files have exactly one `(I)(F)` ACE for the user. It passes.
- End to end, with `SAFESH_STATE_DIR=C:\temp\item4state`: `C:\temp` grants full control to `joel1`, to another local user (`xjchip`), to Administrators and to SYSTEM, and the state root inherits all four. After `hook codex` writes an approval record, `approvals` has only `joel1:(OI)(CI)(F)` and the record has only `joel1:(I)(F)`.
- All 27 Windows unit tests pass, and the item 3 checks still pass with the moved kill code.
- macOS: all 707 tests pass, and clippy is clean on macOS and for `aarch64-pc-windows-msvc`.

## 8. Item 5 fixed

Scratchpad trust now works on Windows. The layout comes from Claude Code 2.1.251's Windows build (`claude.exe` on Legion). The scratchpad is `realpath(CLAUDE_CODE_TMPDIR or os.tmpdir()) + \claude\<project>\<session>\scratchpad`. `<project>` is the cwd with every non-alphanumeric char replaced by `-`, cut at 200 chars plus a hash. Node has no `getuid` on Windows, so the root is `claude`, not `claude-<uid>`, and Claude Code skips its own ownership check there. A failed run on Legion created `%TEMP%\claude`, which confirms the root.

- **Lexical check:** `is_under_windows_root` checks that the path sits below `<root>\<project>\<session>\scratchpad\` with something after it. Names compare without case, either separator works, `\\?\` is dropped and `..` is resolved first. It is pure string code, unit-tested on every platform.
- **Root:** `TEMP`, then `TMP` (Node's order), with a Git Bash spelling mapped, resolved to its real path, plus `\claude`. Like Unix, `CLAUDE_CODE_TMPDIR` is not honored, so a session that sets it gets no trust.
- **Owner check:** the root must be a real dir, not a symlink or junction, owned by the process user, `BUILTIN\Administrators` or `SYSTEM`. An elevated session's new objects belong to Administrators: Legion's `%TEMP%\claude` is owned by Administrators. Admins and SYSTEM already control the machine, so accepting them gives nothing away. This is `compat::dir_owned_by_user_or_admins` (`GetNamedSecurityInfoW`, `EqualSid`, `IsWellKnownSid`).
- The existing-file and new-file paths, real-path re-check included, are unchanged and work on Windows paths.

Verified on Legion in Git Bash, through `hook claude` then `run`, no user config. The baseline is the item 4 build.

| Case | Fixed | Baseline |
|---|---|---|
| `<pad>\who.exe`, Windows spelling | allow, runs | deny |
| `/tmp/claude/.../scratchpad/who.exe`, Git Bash spelling | allow, runs | deny |
| the same exe with another session id | deny | deny |
| `who.exe` in `%TEMP%\item5-notpad` | deny | deny |
| `cp ... <pad>/new.exe && <pad>/new.exe` | allow, runs | deny |

The test uses a copy of `whoami.exe`, because `#!` scripts don't start under brush on Windows (see "Still open"). Unit tests on Windows: the owner check accepts a fresh temp dir and rejects a file, a missing path and a junction. All 11 `safesh-policy` lib tests pass. macOS: all 710 tests pass, and clippy is clean on macOS and for `aarch64-pc-windows-msvc`.

Not tested: a root owned by another non-admin user. Making one needs a second account to create it.

## 9. Item 6 fixed

The `gui` channel now works on Windows: a Windows Forms dialog run by Windows PowerShell 5.1. It is the same channel name, so the default order `ask`, `tmux`, `gui` picks it up. Its design matches `osascript` and `zenity`: an external process with a fixed script, where agent text is only ever data.

- **Dialog:** a fixed title "safesh", the request title in bold, the body (commands or path, and cwd) in a read-only text box, one radio button per choice with Deny preselected, then OK and Cancel. OK prints the picked label. Cancel, Esc and the close box print `false`, which is Deny, as on macOS. At the deadline `proc::run` kills `powershell.exe`, and the dialog goes with it.
- **Script:** fixed, sent with `-EncodedCommand` (base64 of UTF-16LE), with `-NoProfile -NonInteractive`. The title, body, labels and default index arrive in `SAFESH_DIALOG_*` env vars. The script calls .NET directly and uses no cmdlets, so nothing autoloads from `PSModulePath`; a unit test checks for Verb-Noun names.
- **Env:** the child starts with an empty env plus `SystemRoot`, `windir` and the dialog data. At runtime the env comes from the agent's shell, which could set `PSModulePath`, `COR_ENABLE_PROFILING`/`COR_PROFILER_PATH` and the like.
- **Binary:** `powershell.exe` is looked up only in `<system dir>\WindowsPowerShell\v1.0`. The system dir comes from `GetSystemDirectoryW` (`compat::system_dir`), not `SystemRoot` or `PATH`. `CREATE_NO_WINDOW` keeps a console from flashing up.
- **When it is offered:** `SESSIONNAME` set (`Console`, or `RDP-Tcp#N`) and no `SSH_CONNECTION`, `SSH_CLIENT` or `SSH_TTY`. Explorer gets `SESSIONNAME` from `HKCU\Volatile Environment\<session>`, so a Claude Code started from the desktop has it. SSH sessions, services and scheduled tasks don't, and there a dialog would be invisible. They fall back to the text menu.

Verified on Legion, in the logged-on console session, through a scheduled task (`/IT`) running `safesh approve-ask --channels gui`, which grants nothing. The task sets `SESSIONNAME=Console`, because tasks don't get it, and sets a bogus `PSModulePath`. The dialog was driven with `BM_CLICK`. UI Automation reports every Windows Forms control as a Pane here, so its patterns don't apply.

| Case | Result |
|---|---|
| dialog appears | within 0.6 to 3.4 s, all five labels, Deny preselected (window capture) |
| select `3. Allow for session`, click OK | `Session`, 1 s |
| click Cancel | `Deny`, 1 s |
| no answer, `--show-sec 6` | `none` at 6 s, no dialog process left |

Unit tests, all platforms: base64 vectors, hostile text reaching the child only through env with exactly six env vars set, no cmdlets in the script, and availability (desktop session yes; no `SESSIONNAME`, any SSH var, no PowerShell or no Windows dir: no). macOS: all 715 tests pass, and clippy is clean on macOS and for `aarch64-pc-windows-msvc`.

Not tested: a real Claude Code hook prompt through the dialog. The hook calls the same `ask_user` as `approve-ask`, and Claude Code on Legion isn't logged in.

