#!/usr/bin/env bash
# gate <repo>: combine every agent head recorded on the coord board, auto-resolve with
# Mergiraf (pairwise, jj tools take 2 sides), then run the combined tests.
# Exit 0 = PASS (safe to land), 1 = FAIL (report names the failing tests / files).
set -uo pipefail
REPO=$1; cd "$REPO" || exit 2
HEADS=$(python3 -I -c 'import json;print("|".join(sorted(set(json.load(open("/tmp/sw-v2-board/state.json"))["heads"].values()))))')
[ -n "$HEADS" ] || { echo "FAIL no agent heads on board"; exit 1; }
TIPS=$(jj log --no-pager --no-graph -r "heads($HEADS)" -T 'change_id.short() ++ "\n"')
echo "agent heads: ${HEADS//|/ }  ->  tips to combine: $(echo $TIPS)"
set -- $TIPS; cur=$1; shift
jj new "$cur" -m "gate: combined" >/dev/null 2>&1
for t in "$@"; do
  jj new "$cur" "$t" -m "gate: combine $t" >/dev/null 2>&1
  for f in $(jj resolve --list 2>/dev/null | awk '{print $1}'); do
    jj resolve --tool mergiraf "$f" >/dev/null 2>&1 && echo "mergiraf resolved $f" || echo "mergiraf could not resolve $f"
  done
  cur=$(jj log --no-pager --no-graph -r @ -T 'change_id.short()')
done
LEFT=$(jj resolve --list 2>/dev/null | awk '{print $1}')
[ -z "$LEFT" ] || { echo "FAIL unresolved conflicts: $LEFT"; exit 1; }
OUT=$(python3 -m pytest -q -p no:asyncio 2>&1)
echo "$OUT" | grep -E 'FAILED|passed|failed'
echo "$OUT" | grep -qE '[0-9]+ failed|error' && { echo "FAIL combined tests"; exit 1; }
echo "PASS combined tests on $(jj log --no-pager --no-graph -r @ -T 'change_id.short()')"
