# A1 spike results: approval channels

Date: 2026-11-06. Code: `spikes/approval/`. Design: `interactive-approval.md`.
Machine: macOS (Darwin), running inside tmux. Ask test: Claude Code 2.1.318, session `639e1f5d-2e5b-4b0b-8b92-9f7aaf468c9a`, auto permission mode.
Probes ran detached, in a new session with no controlling terminal and piped stdin, the way Claude Code runs hooks.

## Results

| Question | Result |
|---|---|
| Does a hook-like process have a terminal? | No. `tty` reports "not a tty" and `/dev/tty` can't be opened. This matches the Claude Code docs. |
| Are tmux vars inherited? | Yes. `TMUX` and `TMUX_PANE` reach the detached process. |
| Can it open a tmux popup? | **Yes.** `tmux display-popup -E -t "$TMUX_PANE"` showed a popup over the agent's pane, returned 0, and blocked until the popup closed (3.1 s for a 3 s popup). |
| Can it open a macOS dialog? | **Yes.** `osascript` with `display dialog ... giving up after 4` showed a dialog, returned after 4.3 s with `gave up:true`, and exited 0. Command text went in as argv. |
| Linux zenity | Not tested: no Linux desktop here. Run `./run-probes.sh zenity` on one. |
| Claude Code "ask": does the model see `updatedInput`? | **No.** The model got only the output, and said it couldn't see what ran. The token never appears in an API request. It does appear in the on-disk transcript, but only for approved runs. |
| Claude Code "ask": what does "don't ask again" save? | **Nothing.** Each approval was recorded as `user_temporary` with scope `once`. No rule was saved, and run two prompted again. |
| Claude Code "ask" in auto mode | Still prompts the user. Decisions show `reasonCode: hook:PreToolUse`, so the hook's "ask" overrides the auto classifier. |
| Claude Code "ask": what does the model get on "No"? | A generic "user rejected" result telling it to stop and wait. The rewritten command and token are not in it, and not in the transcript. |
| tmux popup that waits for an answer | **Yes.** The popup took typed input from a detached process and returned 0. The choice (`1`) came back through a temp file, and the call blocked until the user answered (9.3 s). |
| macOS list dialog with five choices | **Yes.** `osascript` `choose from list` worked from a detached process and returned 0. It printed the full label of the picked item (`5. Always allow (all projects)`) on stdout, after 5.0 s. On Cancel it prints `false` and still exits 0, so the hook treats `false` as deny. |

## What this means for the design

- The tmux channel works as designed: the hook env has what it needs, and the popup call blocks, so the hook can wait for the answer. A real typed choice makes the round trip through a temp file.
- The macOS channel works from a hook process, and the built-in timeout gives a clean fallback. A five-choice list dialog returns the picked label on stdout, so the hook can map it back to a choice.
- The "ask" channel is safe to use. The model can't see the token, and a declined token is never written anywhere the agent can read. Keep the token rules in section 5 as defense in depth, since an approved token is on disk after it is spent.
