#!/bin/bash
# Reads the spike hook log, the session transcript and the saved permission rules, and
# reports what the "ask" test showed.
dir=/tmp/safesh-ask-spike
log="$dir/hook.log"
echo "== tokens issued"
grep '^issued' "$log" || echo "(none: the hook never answered ask)"
proj="$HOME/.claude/projects/$(printf "%s" "$(cd "$dir" && pwd -P)" | sed 's#[/.]#-#g')"
tx=$(ls -t "$proj"/*.jsonl 2>/dev/null | head -1)
echo "== transcript: ${tx:-not found under $proj}"
if [ -n "$tx" ]; then
  for t in $(grep '^issued' "$log" | awk '{print $2}'); do
    echo "-- $t"
    python3 -I - "$tx" "$t" <<'PY'
import json, sys
path, tok = sys.argv[1], sys.argv[2]
for n, line in enumerate(open(path, errors="replace"), 1):
    if tok not in line:
        continue
    try:
        o = json.loads(line)
    except Exception:
        print(f"  line {n}: (not json)")
        continue
    msg = o.get("message") or {}
    role = msg.get("role") or o.get("type")
    kinds = []
    content = msg.get("content")
    if isinstance(content, list):
        for b in content:
            if isinstance(b, dict) and tok in json.dumps(b):
                kinds.append(b.get("type"))
    elif isinstance(content, str) and tok in content:
        kinds.append("text")
    extra = [k for k in o if k not in ("message",) and tok in json.dumps(o.get(k))]
    print(f"  line {n}: role={role} blocks={kinds} other_fields={extra}")
PY
  done
  echo "== tool results for SPIKE_ASK runs"
  python3 -I - "$tx" <<'PY'
import json, sys
for line in open(sys.argv[1], errors="replace"):
    try:
        o = json.loads(line)
    except Exception:
        continue
    c = (o.get("message") or {}).get("content")
    if not isinstance(c, list):
        continue
    for b in c:
        if isinstance(b, dict) and b.get("type") in ("tool_use", "tool_result"):
            s = json.dumps(b)
            if "SPIKE_ASK" in s or "rewritten-ran" in s or "spike" in s:
                print(" ", b.get("type"), s[:300])
PY
fi
echo "== saved permission rules"
for f in "$dir/.claude/settings.local.json" "$dir/.claude/settings.json"; do
  [ -f "$f" ] && { echo "-- $f"; cat "$f"; echo; }
done
