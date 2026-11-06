# Approval channel spike (A1)

Throwaway probes for `notes/interactive-approval.md`. Results go in `notes/approval-spike-results.md`.

## Automated probes

```sh
./run-probes.sh                # env, tmux popup, macOS dialog, zenity (all close by themselves)
./run-probes.sh tmux-ask       # tmux popup that waits for your answer
./run-probes.sh gui-ask        # macOS list dialog that waits for your answer
```

`run-probes.sh` runs each probe in a new session with no controlling terminal and a piped stdin, which is how Claude Code runs command hooks.

## Claude Code "ask" test (manual)

```sh
./setup-ask.sh                 # creates /tmp/safesh-ask-spike with a project-level spike hook
# follow the printed steps in a separate claude session, then:
./report-ask.sh
```

The spike hook answers `ask` for Bash commands containing `SPIKE_ASK`, with a rewritten command that carries a random token in a comment. The token never appears in the command's output. So if the model can repeat it, the model saw `updatedInput`. `report-ask.sh` also shows where the token appears in the transcript and which permission rule "don't ask again" saved.
