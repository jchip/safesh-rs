#!/bin/bash
# Approval-channel probe. Run it the way Claude Code runs a hook: detached, no controlling tty
# (see run-probes.sh). Appends findings to the log file given as $1.
# Modes: env, tmux (auto-closing popup), gui (auto-dismissing dialog), zenity,
#        tmux-ask and gui-ask (wait for a real answer, for manual runs).
log="$1"
mode="${2:-env}"
say() { printf '%s\n' "$*" >> "$log"; }
now() { python3 -I -c 'import time;print(time.time())'; }
elapsed() { python3 -I -c "print(round($(now)-$1,1))"; }

case "$mode" in
env)
  say "pid=$$ ppid=$PPID"
  say "tty: $(tty 2>&1)"
  if (exec 3</dev/tty) 2>/dev/null; then say "/dev/tty: openable"; else say "/dev/tty: not openable"; fi
  say "TMUX=${TMUX:-} TMUX_PANE=${TMUX_PANE:-} TERM_PROGRAM=${TERM_PROGRAM:-}"
  say "DISPLAY=${DISPLAY:-} WAYLAND_DISPLAY=${WAYLAND_DISPLAY:-} SSH_CONNECTION=${SSH_CONNECTION:+set}"
  say "uname=$(uname -s) osascript=$(command -v osascript || echo none) zenity=$(command -v zenity || echo none) kdialog=$(command -v kdialog || echo none)"
  ;;
tmux)
  if [ -z "$TMUX_PANE" ]; then say "tmux: skipped, TMUX_PANE unset"; exit 0; fi
  t=$(now)
  tmux display-popup -E -t "$TMUX_PANE" -w 56 -h 6 -T " safesh spike " -- \
    sh -c 'echo "safesh approval spike: this popup closes in 3 s"; sleep 3'
  rc=$?
  say "tmux popup: rc=$rc elapsed=$(elapsed "$t")s (about 3 s means it blocks until closed)"
  ;;
tmux-ask)
  if [ -z "$TMUX_PANE" ]; then say "tmux-ask: skipped, TMUX_PANE unset"; exit 0; fi
  ans=$(mktemp)
  t=$(now)
  tmux display-popup -E -t "$TMUX_PANE" -w 60 -h 10 -T " safesh approval " -- \
    sh -c 'printf "safesh spike wants approval for: rm build\n\n1 once  2 project  3 session  4 deny  5 global\n\nchoice: "; read -r a; printf "%s" "$a" > "$1"' sh "$ans"
  say "tmux-ask: rc=$? answer=$(cat "$ans") elapsed=$(elapsed "$t")s"
  rm -f "$ans"
  ;;
gui)
  if [ "$(uname -s)" != Darwin ]; then say "gui: skipped, not macOS"; exit 0; fi
  t=$(now)
  out=$(osascript -e 'on run argv' \
    -e 'display dialog (item 1 of argv) with title "safesh spike" buttons {"Deny", "Allow once"} default button 2 giving up after 4' \
    -e 'end run' "safesh approval spike. This dialog closes by itself in 4 s." 2>&1)
  say "gui dialog: rc=$? out=[$out] elapsed=$(elapsed "$t")s"
  ;;
gui-ask)
  if [ "$(uname -s)" != Darwin ]; then say "gui-ask: skipped, not macOS"; exit 0; fi
  t=$(now)
  out=$(osascript -e 'on run argv' \
    -e 'choose from list {"1. Allow once", "2. Always allow (this project)", "3. Allow for session", "4. Deny", "5. Always allow (all projects)"} with title "safesh" with prompt (item 1 of argv) default items {"4. Deny"}' \
    -e 'end run' "safesh spike: allow 'rm build' in ~/proj?" 2>&1)
  say "gui-ask: rc=$? out=[$out] elapsed=$(elapsed "$t")s"
  ;;
zenity)
  if ! command -v zenity >/dev/null; then say "zenity: skipped, not installed"; exit 0; fi
  if [ -z "$DISPLAY$WAYLAND_DISPLAY" ]; then say "zenity: skipped, no display"; exit 0; fi
  t=$(now)
  out=$(zenity --list --radiolist --timeout=4 --title "safesh spike" --text "closes in 4 s" \
    --column pick --column choice TRUE "1 once" FALSE "4 deny" 2>&1)
  say "zenity: rc=$? out=[$out] elapsed=$(elapsed "$t")s (rc 5 = timeout)"
  ;;
*)
  say "unknown mode $mode"
  exit 2
  ;;
esac
