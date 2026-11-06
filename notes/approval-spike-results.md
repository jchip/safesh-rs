# A1 spike results: approval channels

Date: 2026-11-06. Code: `spikes/approval/`. Design: `interactive-approval.md`.
Machine: macOS (Darwin), running inside tmux. Probes ran detached, in a new session with no controlling terminal and piped stdin, the way Claude Code runs hooks.

## Results

| Question | Result |
|---|---|
| Does a hook-like process have a terminal? | No. `tty` reports "not a tty" and `/dev/tty` can't be opened. This matches the Claude Code docs. |
| Are tmux vars inherited? | Yes. `TMUX` and `TMUX_PANE` reach the detached process. |
| Can it open a tmux popup? | **Yes.** `tmux display-popup -E -t "$TMUX_PANE"` showed a popup over the agent's pane, returned 0, and blocked until the popup closed (3.1 s for a 3 s popup). |
| Can it open a macOS dialog? | **Yes.** `osascript` with `display dialog ... giving up after 4` showed a dialog, returned after 4.3 s with `gave up:true`, and exited 0. Command text went in as argv. |
| Linux zenity | Not tested: no Linux desktop here. Run `./run-probes.sh zenity` on one. |
| Claude Code "ask": does the model see `updatedInput`? | Pending: needs an interactive session. Run `setup-ask.sh`, then `report-ask.sh`. |
| Claude Code "ask": what does "don't ask again" save? | Pending: same test. |
| Claude Code "ask": what does the model get on "No"? | Pending: same test. |
| tmux popup that waits for an answer | Pending: run `./run-probes.sh tmux-ask` and type a choice. |
| macOS list dialog with five choices | Pending: run `./run-probes.sh gui-ask` and pick one. |

## What this means for the design

- The tmux channel works as designed: the hook env has what it needs, and the popup call blocks, so the hook can wait for the answer.
- The macOS channel works from a hook process, and the built-in timeout gives a clean fallback.
- The "ask" channel depends on the pending test. If the model sees the token, keep the token rules in design section 5, and treat a declined prompt as replayable until the token expires.
