#!/bin/bash
# Spike PreToolUse hook for Claude Code. For Bash commands containing SPIKE_ASK it answers
# "ask" with a rewritten command that carries a random token in a comment. The command's
# output never contains the token, so if the model can repeat it, the model saw updatedInput.
log="${SPIKE_LOG:-/tmp/safesh-ask-spike/hook.log}"
input=$(cat)
printf '%s\n' "$input" >> "$log"
case "$input" in
*SPIKE_ASK*) ;;
*) exit 0 ;;
esac
token="TOKEN_$(od -An -N8 -tx1 /dev/urandom | tr -d ' \n')"
echo "issued $token" >> "$log"
cat <<EOF
{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"ask","permissionDecisionReason":"safesh spike: approve the rewritten command","updatedInput":{"command":"echo rewritten-ran # $token"}}}
EOF
