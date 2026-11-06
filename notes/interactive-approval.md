# Interactive approval: design

Status: channels approved 2026-11-06 (ask, tmux, gui). A1 spike in progress: see `approval-spike-results.md`.
Related: `design.md` sections 5 and 6, `architecture.md` sections 2 and 5.

## 1. Goal

Today a blocked command goes back to the agent as a deny message with a 1-5 menu. The agent shows the menu, waits for the user, then runs `safesh retry`. That costs a round trip through the model, and the agent can garble or skip the menu.

The goal is to ask the user directly when safesh can reach them, and fall back to today's text when it can't.

## 2. What is possible

| Facts that shape the design | Source |
|---|---|
| Claude Code hooks run in their own session with no controlling terminal. They can't open `/dev/tty` or write escape sequences to the Claude Code UI. | Claude Code hooks docs |
| A PreToolUse hook may return `permissionDecision: "ask"`. Claude Code then shows its own permission prompt, in its own terminal. Combined with `updatedInput`, the prompt shows the rewritten input. `permissionDecisionReason` is shown to the user. | Claude Code hooks docs |
| The default hook timeout is 600 s, and it is configurable per hook. Our Codex installer sets 30 s. | Claude Code hooks docs, `safesh install` |
| `safesh run` runs inside the agent's Bash tool. Its stdout and stderr are captured, and it has no terminal to ask on. | observed |
| Gemini CLI and Codex support for an "ask" decision is not confirmed. | docs unclear |

So a popup that safesh draws inside the agent's terminal is not possible. Two things come close:
- **Agent-native prompt:** Claude Code draws the prompt itself when the hook says "ask". This is the same-terminal experience, but only for Claude Code and only before the run.
- **tmux popup:** when the agent runs inside tmux, `tmux display-popup` opens a box over the same window. It is drawn by tmux, not by safesh, so the no-terminal limit doesn't apply.

## 3. Approval channels

safesh tries channels in order and uses the first one that is available. If none answers, it falls back to text.

| Order | Channel | Available when | Stage |
|---|---|---|---|
| 1 | Agent-native "ask" | the hook runs as `safesh hook claude` | pre-check only |
| 2 | tmux popup | `$TMUX` and `$TMUX_PANE` are set in the hook env | pre-check and runtime |
| 3 | GUI dialog | macOS, not over SSH; Linux with `$DISPLAY` or `$WAYLAND_DISPLAY` and `zenity` or `kdialog` | pre-check and runtime |
| 4 | Text (today) | always | pre-check and runtime |

Channel details:
- **Agent-native:** see section 5. The agent comes from the hook's argv, which `safesh install` writes, so detection never guesses from env vars or stdin fields. For `codex`, `gemini` and `agy`, safesh skips this channel.
- **tmux:** `tmux display-popup -E -t "$TMUX_PANE" -- safesh approve-ui --id <pending>`. The popup runs a small safesh TUI that reads one choice and writes it to the pending file. Kitty, WezTerm and Zellij floating panes are possible later channels of the same kind.
- **macOS GUI:** `osascript` with `choose from list` for the five choices (`display dialog` allows only three buttons). The command text goes in as an argv item (`on run argv`), never spliced into AppleScript source.
- **Linux GUI:** `zenity --list --radiolist`, or `kdialog --radiolist`.

## 4. Where approval happens

### 4.1 Pre-check (hook)

When the pre-check finds problems, the hook writes the pending file as today, then tries a channel before it answers the agent.

| User answer | Hook result |
|---|---|
| 1 once | rewrite to `safesh run`, with a once-grant for this run (4.3) |
| 2 project, 3 session, 5 global | save the grant as `retry` does, then rewrite to `safesh run` |
| 4 deny | deny with "denied by the user", and no retry menu |
| timeout, no channel, channel error | deny with today's retry menu |

The agent never sees the menu when a channel answers. It just sees the command run, or a plain denial.

### 4.2 Runtime gate (`safesh run`)

A runtime denial today stops the script with status 126. Then `retry` reruns the script from the top, so commands before the denial run twice.

With a channel, the gate asks first:
- On approval, the exec or open goes ahead and the script keeps running. There is no abort and no rerun. A persistent scope is saved as `retry` saves it.
- On deny or timeout, the gate aborts with today's text.

The agent's Bash tool has its own timeout (Claude Code defaults to 2 minutes). The runtime prompt timeout must stay well under it.

### 4.3 Once-grants

"Allow once" must cover exactly one run of one script.
- **Pre-check:** the hook stores the approved commands or paths in the pending file and marks it `approved_once`. `safesh run --id <script> --pending <id>` applies them, then deletes the pending file. A second run with the same pending id finds nothing.
- **Runtime:** the grant lives in the running gate's memory only.

## 5. Agent-native "ask" (Claude Code)

The hook returns:

```json
{ "hookSpecificOutput": {
    "hookEventName": "PreToolUse",
    "permissionDecision": "ask",
    "permissionDecisionReason": "safesh: `foo` is not on the allowlist",
    "updatedInput": { "command": "'/path/safesh' run --id <script> --pending <id> --token <t>" } } }
```

Claude Code shows its normal prompt with the reason and the rewritten command. If the user approves, the rewritten command runs and applies the once-grant. If the user declines, nothing runs, and the pending file expires after 5 minutes.

Limits:
- It only offers "allow once". Persistent scopes still go through `safesh retry` or a GUI or tmux channel.
- Claude Code records the approval as once-only and saves no rule (spike result). Each new command prompts again.

Replay risk: if the user declines, the agent could try to run the same `safesh run --pending ... --token ...` command itself.
- The token is 128 random bits, stored only in the pending file, single use, bound to the session id, and valid for 5 minutes.
- `safesh run` with `--pending` and `--token` is no longer control-plane passthrough. The hook treats it as a normal command and requires that the hook itself issued the token to this session in an "ask" answer.
- The model does not see `updatedInput` (spike result). A declined token is never written to the transcript. An approved one is, but it is already spent.

## 6. Security

The approval must come from the user, never from the agent.

| Risk | Mitigation |
|---|---|
| Agent answers a tmux popup with `tmux send-keys` | Default argv rules ask for `tmux send-keys`, `tmux paste-buffer` and `tmux capture-pane`. `tmux` itself stays allowed. |
| Agent clicks a GUI dialog with `osascript`, `xdotool` or `cliclick` | They are not on the allowlist. A rule denies them outright while a prompt is open. |
| Agent writes the pending file to mark it approved | The pending file holds no approval state for channels. The channel writes its answer to a separate per-prompt file under `<state>/approvals-ui/`, mode 0600, with a nonce the hook checks. `<state>` must be on the deny-write list (section 6.1). |
| Agent-controlled text in the prompt (command, paths) tricks the user | Show the text plain, without markup, truncated with a "+N more" note. The fixed safesh header and the choice labels are never built from agent text. |
| AppleScript or shell injection through the command text | Pass the text as argv or stdin only. Never splice it into script source. |
| Many prompts at once (parallel tool calls) | One prompt at a time per user, behind a lock file. Others wait in a queue. If the wait would pass the timeout, fall back to text. |
| No answer | Timeout means deny-with-text. It never means allow. |

### 6.1 Policy files (fixed 2026-11-06)

Default write access used to cover files that hold grants: `<state>` (pending files and Codex approvals, under `/tmp`), session grants in `<project>/.temp/safesh`, and project config in `<project>/.config/safesh`. An agent could write them and grant itself access. TS has the same gap.

Now these are on a built-in deny-write list, together with the global config dir, for each workspace root and the cwd. `safesh retry`, `install` and the hook write them from outside the gate, so they still work.

Limits:
- A write naming a protected file is denied. A directory-level write into an ancestor, such as `tar -x -C .` or `cp -r x .`, can still place files inside one.
- Programs safesh doesn't see, such as a Python script or `git checkout`, can still write them, as with every other path (`design.md` section 3).

## 7. Config

```json
{
  "approval": {
    "channels": ["ask", "tmux", "gui"],
    "timeoutSec": 60,
    "runtimeTimeoutSec": 45
  }
}
```

- `channels`: the order to try. `[]` means text only, which is today's behavior.
- `SAFESH_APPROVAL=off` forces text only, for CI and headless runs.
- Default: `["ask", "tmux", "gui"]` (approved 2026-11-06).

## 8. Plan

| Step | Work | Exit check |
|---|---|---|
| A1 spike | From a real hook process, verify: tmux popup with `-t $TMUX_PANE`; macOS `osascript` dialog; Linux zenity; "ask" with `updatedInput`, what the model sees, and what "don't ask again" saves | a short results note with each fact confirmed or refuted |
| A2 | Channel trait, GUI and tmux channels, `approve-ui` TUI, lock and queue, answer files | unit tests with a fake channel; manual test on macOS and Linux |
| A3 | Pre-check uses channels (4.1) and once-grants (4.3) | e2e tests with a fake channel binary |
| A4 | Runtime gate uses channels (4.2) | e2e test: approved runtime exec continues without a rerun |
| A5 | Claude Code "ask" channel with tokens (section 5) | e2e test of the token rules; manual test in Claude Code |
| A6 | Default argv rules for tmux and GUI automation tools | rules tests |
| A0 | Deny-write for policy files (6.1) | done 2026-11-06 |

## 9. Decisions needed

1. ~~Default `channels`~~: `["ask", "tmux", "gui"]`, approved 2026-11-06.
2. ~~"ask" replay risk~~: acceptable. The spike showed the model never sees the token.
3. ~~Popup scopes~~: all five, approved 2026-11-06.
4. ~~Prompt timeouts~~: 60 s at the pre-check and 45 s at runtime, approved 2026-11-06. safesh enforces them with its own timer for every channel.
