# A2 plan: approval channels

Design: `interactive-approval.md` sections 3, 6 and 7. Spike: `approval-spike-results.md`.

## Scope

A2 builds the channels and proves they return a real answer. It does not wire them into the hook (A3) or the runtime gate (A4). The "ask" channel is hook output, not a popup, so it waits for A5.

In scope:
- A `Channel` trait, plus tmux, macOS and Linux GUI channels.
- `safesh approve-ui`, the small picker that runs inside the tmux popup.
- Answer files with a nonce, and a queue so prompts show one at a time, in arrival order.
- The `approval` config block and `SAFESH_APPROVAL=off`.

## Shape

All new code lives in `safesh-cli/src/approval/`. That crate already owns retry, gate, hook and context. The policy and hooks crates don't change, except that `approval` config parsing goes next to the other config code in `safesh-policy`.

```rust
pub enum Request { Command { commands: Vec<String> }, Path { path: String, op: PathOp } }
pub struct Prompt { pub id: String, pub cwd: String, pub request: Request }
pub enum Choice { Once, Project, Session, Global, Deny }

pub trait Channel {
    fn available(&self, env: &Env) -> bool;
    /// None means no answer: timeout, closed popup, or channel error.
    fn ask(&self, env: &Env, prompt: &Prompt, deadline: Instant) -> Option<Choice>;
}

pub struct Timing { pub show: Duration, pub give_up_at: Instant }
pub fn ask_user(order: &[String], env: &Env, prompt: &Prompt, timing: Timing) -> Option<Choice>;
```

`ask_user` checks `SAFESH_APPROVAL`, waits for its turn in the queue, then asks the first available channel in config order. Only one channel is tried. A `None` from it means text fallback, never allow.

`show` is the time the user gets once the prompt is on screen (60 s or 45 s). It starts when the prompt shows, not when it joins the queue. `give_up_at` is a hard stop on queue wait plus showing. Each caller sets it to fit its own outer timeout: the agent's hook timeout for the pre-check (A3), and its Bash tool timeout at runtime (A4).

## Channels

| Channel | How | How the answer comes back |
|---|---|---|
| tmux | `tmux display-popup -E -t $TMUX_PANE -- <safesh> approve-ui --id <id>` | answer file written by `approve-ui` |
| macOS | `osascript` `choose from list`, prompt text as argv | stdout label; `false` means deny |
| Linux | `zenity --list --radiolist`, else `kdialog --radiolist` | stdout |

Each channel runs as a child process. safesh kills it at the deadline, since `choose from list` has no timeout of its own. On a tmux timeout safesh also runs `display-popup -C`, and `approve-ui` exits at the same deadline.

Channel binaries are looked up only in `/usr/bin`, `/bin`, `/opt/homebrew/bin` and `/usr/local/bin`, never `$PATH`. At runtime the env comes from the agent's shell, which could put a fake `osascript` first that always answers "allow".

Path requests show four choices: once, session, project, deny. That matches what `retry-path` supports today, minus the read/write and directory options. The operation comes from the request.

## Answer files and queue

- Dir: `<state>/approvals-ui/`, mode 0700. It is under `<state>`, which is already on the deny-write list.
- `<id>.prompt.json` holds the prompt and a 128-bit nonce. `approve-ui` reads it, shows it, and writes `<id>.answer.json` with the choice and the nonce. Both files are 0600.
- The caller accepts the answer only if the nonce matches, then deletes both files.
- Queue: each prompt creates a ticket `<state>/approvals-ui/queue/<nanos>-<pid>-<id>`. A prompt shows when its ticket sorts first. Tickets whose process died are removed. A waiter gives up at `give_up_at` and falls back to text.
- Lock: `<state>/approvals-ui/lock`, held with `File::lock` while a prompt shows. It backs up the ticket order, and the OS frees it on a crash.

## Prompt text

The header and choice labels are fixed strings. The command or path is shown plain, cut at a few lines with a "+N more" note. It goes to `osascript` and `zenity` as argv, and to `approve-ui` through the prompt file, never through a shell string.

## Tests

- Unit: choice label mapping, `false` as deny, argv building with hostile text (quotes, newlines, AppleScript keywords), truncation, config parsing, `SAFESH_APPROVAL=off`.
- Integration (`tests/e2e_approval.rs`) through the hidden `safesh approve-ask` command, with fake `tmux`, `osascript` and `zenity` in a temp bin dir: answer round trip, forged nonce rejected, show timeout, queue order, queue give-up, stale ticket, `SAFESH_APPROVAL=off`.
- Manual: run `safesh approve-ask` for a real tmux popup and a real macOS dialog. Linux zenity stays untested until a Linux desktop is available.
- Manual results (2026-11-06, macOS, tmux 3.8): tmux command prompt returned `Once`. macOS path dialog returned `Once`. Two queued tmux prompts showed one after the other and returned `Project` and `Session`.

## Work split

| Task | Who |
|---|---|
| Trait, `ask_user`, lock, answer files, deadline handling | main session |
| `approve-ui` picker | main session |
| `approval` config parsing and its tests | Sonnet subagent |
| osascript and zenity argv builders, label mapping, and their tests | Sonnet subagent |
| Fake channel binaries and the integration tests | Sonnet subagent |

## Decisions

1. Raise `rust-version` to 1.89 so the lock can use `File::lock`, which the OS releases on a crash.
2. Ship the Linux channel in A2 with argv tests only. It stays marked untested until someone runs it on a Linux desktop.
3. Prompts queue in arrival order. Each one's show timeout starts when it appears.
