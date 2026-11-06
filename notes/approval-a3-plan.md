# A3 plan: the pre-check asks the user

Design: `interactive-approval.md` sections 4.1 and 4.3. Channels: `approval-a2-plan.md`.

## Flow

When the pre-check blocks a command or path that the user can approve, the hook writes the pending file as today, then calls `ask_user` before it answers the agent.

| Answer | Hook result |
|---|---|
| once | write a once-grant, rewrite to `safesh run --id <script> --pending <id>` |
| project, session, global | save the grant as `retry` does, delete the pending file, rewrite to `safesh run --id <script>` |
| deny | delete the pending file, deny with "denied by the user", no menu |
| no answer | today's text menu |

Hard denials (deny lists, deny rules) never ask. Session without a session id becomes once, as in `retry`. For Codex, every rewrite also writes the approval record, as today.

## Once-grants

- File: `<state>/once/<pending id>.json` with the script id and the granted commands, or the path and its operation. Mode 0600. `<state>` is on the deny-write list.
- The pending file holds no approval state (design section 6).
- `safesh run --id <script> --pending <id>` claims the file by rename, checks that the script id matches, applies the grant for this run, and deletes it. A second run finds nothing.
- `safesh run` with `--pending` is not control-plane passthrough. When the agent types it, the pre-check checks it like any other command.

## Time budget

- `show` is `approval.timeoutSec` (60 s).
- `give_up_at` is now plus the hook budget: `--budget-sec` on the hook command.
- `safesh install` sets the hook timeout to 600 s and adds `--budget-sec 590`.
- Without the flag: 590 s for Claude (its default hook timeout is 600 s), and 25 s for Codex and agy (old installs set 30 s). Old installs keep working, just with shorter prompts until reinstalled.

## Code

- `retry.rs`: move grant saving into a shared function that the hook also calls.
- `approval/once.rs`: write, claim and apply once-grants.
- `hook.rs`: ask after writing pending, then map the answer to an outcome.
- `main.rs`: `run --pending`, hook `--budget-sec`.
- `safesh-analyze/src/control.rs`: `run` with `--pending` is not passthrough.
- `install.rs`: timeout 600 and `--budget-sec 590` for Claude, Codex and agy.
- Tests: debug builds read `SAFESH_TEST_BIN_DIRS` for channel binaries, so e2e tests can use fake `tmux`. Release builds ignore it.

## Tests

- e2e hook with fake tmux, one per answer: once runs once and a second run is blocked again; project, session and global save the grant; deny gives the plain denial; no answer gives the menu.
- Path requests: once and session.
- Once-grant with the wrong script id is rejected.
- `safesh run --pending` typed by the agent is checked, not passed through.
- Installer output has the timeout and the flag. A hook without the flag uses the per-agent default.

## Work split

| Task | Who |
|---|---|
| Shared grant saving, once-grants, hook wiring, `run --pending` | main session |
| `control.rs` change and its tests | Sonnet subagent |
| Installer timeout and flag, and its tests | Sonnet subagent |
| e2e hook tests | Sonnet subagent, after the core lands |

## Results

- 15 e2e tests in `tests/e2e_hook_approval.rs` cover every answer, no answer, `SAFESH_APPROVAL=off`, shadow, hard denials, a path write, a wrong-script once-grant, the budget cut, the default budget, and two parallel hooks. The workspace has 692 passing tests.
- The parallel test starts two hook processes at once with a slow fake popup. It checks that the popups never overlap. It fails when the queue is disabled.
- Path reads outside the workspace are allowed by default, so the path test uses a write.
- The shared test helper sets `SAFESH_APPROVAL=off`, so other tests never open a real dialog. Before this, the hook found `/usr/bin/osascript` during tests and opened real macOS dialogs.
- An agent can type `safesh run --id <script> --pending <id>`, and the pre-check allows it. It only works when the user approved that exact script in a popup. The grant is single-use, and the hook's own rewrite uses it right away. The pending id the agent sees in a text menu never has a grant. So the worst case is one run of a script the user already approved.

## Live test (2026-11-06)

Claude Code in a throwaway project ran only the Rust hook with `--budget-sec 590`. Every step behaved as expected.

| Step | Popup answer | Result |
|---|---|---|
| `uuidgen` | once | ran |
| `uuidgen` again | deny | "DENIED by the user", no menu |
| `sw_vers` | session | ran |
| `sw_vers` again | none | ran, no popup, hook took 35 ms |
| `caffeinate -t 1` | ignored | text menu after 60 s |
| `uuidgen` and `caffeinate -t 1` in parallel | once, once | second popup only after the first answer |

- Once-grants are used up. The parallel `uuidgen` asked again.
- A timeout saves nothing. The parallel `caffeinate` asked again.
- Claude Code ran the two parallel PreToolUse hooks one after another. The pending ids (`<millis>-<pid>`) show that the second hook process started after the first one returned. So the live run did not test the prompt queue. The parallel e2e test covers it.
