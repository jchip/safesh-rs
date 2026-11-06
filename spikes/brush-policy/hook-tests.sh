#!/bin/bash
# Hook coverage tests for the brush policy spike.
S=/Users/joel.chen/dev/safesh-rs/.temp/brush-spike
B=$S/policy-target/release/brush-policy-spike
W=$S/work
O=$S/outside
rm -rf "$W" "$O"; mkdir -p "$W/sub" "$O"; cd "$W" || exit 1
export SAFESH_SPIKE_ROOTS="$W:/dev"

reset() { for f in f1 f2 f3 f4 f5 f6 f7 f8; do echo x > "$W/$f"; done; }
t() {
  local label=$1; shift
  reset
  printf '%-24s ' "$label"
  "$@" >/dev/null 2>"$S/err.txt"; local rc=$?
  local msg; msg=$(grep -m1 BLOCKED "$S/err.txt" || echo '(no block)')
  local gone; gone=$(for f in f1 f2 f3 f6 f7 f8; do [ -e "$W/$f" ] || printf '%s ' "$f"; done)
  printf 'rc=%-3s %-70.70s deleted:[%s]\n' "$rc" "$msg" "$gone"
}

echo 'rm f6' > "$W/s.sh"
printf 'all:\n\trm -f f7\n' > "$W/Makefile"

t "for loop rm"         "$B" -c 'for f in f1 f2; do rm $f; done'
t 'rm in $( )'          "$B" -c 'x=$(rm f1; echo done)'
t 'x=rm; $x'            "$B" -c 'x=rm; $x f1'
t "eval rm"             "$B" -c 'eval "rm f1"'
t "bash -c rm"          "$B" -c 'bash -c "rm f1"'
t "sh script.sh"        "$B" -c 'sh s.sh'
t "exec rm"             "$B" -c 'exec rm f1'
t "pipeline rm"         "$B" -c 'echo f1 | while read f; do rm $f; done'
t "function rm"         "$B" -c 'del() { rm "$@"; }; del f1'
t "redirect outside"    "$B" -c "echo hi > $O/x.txt"
t "redirect &> outside" "$B" -c "echo hi &> $O/y.txt"
t "redirect inside"     "$B" -c 'echo hi > sub/ok.txt'
t "touch outside"       env SAFESH_SPIKE_DENY= "$B" -c "touch $O/t.txt"
t "mv ../ escape"       env SAFESH_SPIKE_DENY= "$B" -c 'cd sub && mv ../f1 ../../outside/'
t "touch inside"        env SAFESH_SPIKE_DENY= "$B" -c 'touch sub/new.txt'
t "git push --force"    "$B" -c 'git push --force origin main'
t "no-abort continues"  env SAFESH_SPIKE_ABORT=0 "$B" -c 'rm f1; echo after > after.txt'
t "xargs rm"            "$B" -c 'echo f2 | xargs rm'
t "find -exec rm"       "$B" -c 'find . -name f3 -exec rm {} \;'
t "make rm"             "$B" -c 'make -s'
t "python os.remove"    "$B" -c 'python3 -c "import os; os.remove(\"f8\")"'

echo
echo "after.txt exists (no-abort ran next cmd): $([ -e "$W/after.txt" ] && echo yes || echo no)"
echo "outside dir: [$(ls "$O")]"
