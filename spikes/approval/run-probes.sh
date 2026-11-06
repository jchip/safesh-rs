#!/bin/bash
# Runs probe.sh modes detached from the terminal (new session, no controlling tty, stdin
# from a pipe), which is how Claude Code runs command hooks. Prints the log.
#   run-probes.sh                 env, tmux and gui (both auto-close)
#   run-probes.sh tmux-ask        wait for a real answer in a tmux popup
here="$(cd "$(dirname "$0")" && pwd)"
log="$(mktemp)"
modes=("$@")
[ ${#modes[@]} -eq 0 ] && modes=(env tmux gui zenity)
for m in "${modes[@]}"; do
  echo '{"hook_event_name":"PreToolUse"}' | python3 -I -c '
import os, sys
os.setsid()
os.execvp(sys.argv[1], sys.argv[1:])
' "$here/probe.sh" "$log" "$m"
done
cat "$log"
rm -f "$log"
