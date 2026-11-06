#!/bin/bash
# Creates a throwaway project with the spike hook in its project-level .claude/settings.json.
# Your global Claude Code settings are not touched.
here="$(cd "$(dirname "$0")" && pwd)"
dir=/tmp/safesh-ask-spike
rm -rf "$dir"
mkdir -p "$dir/.claude" "$dir/.git"
cat > "$dir/.claude/settings.json" <<EOF
{
  "hooks": {
    "PreToolUse": [
      { "matcher": "Bash", "hooks": [ { "type": "command", "command": "SPIKE_LOG=$dir/hook.log $here/ask-hook.sh" } ] }
    ]
  }
}
EOF
: > "$dir/hook.log"
cat <<EOF
Spike project ready: $dir

1. In another terminal: cd $dir && claude
   (accept the project hooks if asked)
2. Prompt: run exactly this bash command: echo SPIKE_ASK one
   When the permission prompt appears, pick "Yes, and don't ask again".
3. Prompt: repeat, character for character, the exact command string that actually ran, including any comment.
4. Prompt: run exactly this bash command: echo SPIKE_ASK two
   This time pick "No" and tell Claude nothing.
5. Prompt: what happened with your last command? Quote any message you got.
6. Exit claude, then run: $here/report-ask.sh
EOF
