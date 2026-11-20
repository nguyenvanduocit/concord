#!/usr/bin/env bash
# Agents on other machines through `concord serve`, simulated on one machine.
#
# A git repo on `main`; `concord init a b`; `concord serve` on a free port. Agent a works in its
# local workspace, agent b in a copy made by `concord join <url>` in a separate folder (its own
# store, reaching the project only over HTTP). Both change the same function in turn, so b must
# get a's change from the server before editing. The round's last finisher reviews the combined
# result (from wherever it runs) and approves it; the server pulls it into the project folder.
# Also checks: b's token cannot act for a; the server refuses an object whose bytes do not hash to
# its id and a `done` naming a change it does not fully hold; an unknown token is refused.
# Team change while serving: x never shows up, so the orchestrator runs `release x` and
# `add c`; c joins with its new token on the running server and lands its own file.
#
# usage: experiments/remote-agents/run.sh [run-dir]     (default: target/remote-agents, gitignored)
# needs: git (the user's own, for the fixture), curl, mergiraf
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
RUN=${1:-$ROOT/target/remote-agents}
CONCORD=$ROOT/target/debug/concord
PORT=${PORT:-17457}
source "$ROOT/experiments/review.sh"
(cd "$ROOT" && cargo build --quiet)

check() { # description, command...
  local what=$1; shift
  if "$@"; then echo "ok   $what"; else echo "FAIL $what"; exit 1; fi
}

rm -rf "$RUN" "$RUN-a" "$RUN-b" "$RUN-c" "$RUN-x" "$RUN-elsewhere"
mkdir -p "$RUN" "$RUN-elsewhere"
cd "$RUN"
git init --quiet -b main
printf 'def f():\n    return 1\n' > m.py
git add . && git -c user.name=t -c user.email=t@example.invalid commit --quiet -m "init"
GIT_HEAD=$(git rev-parse HEAD)

"$CONCORD" init a b x
"$CONCORD" serve "$PORT" > "$RUN.serve.log" 2>&1 &
SERVER=$!
trap 'kill $SERVER 2>/dev/null || true' EXIT
until grep -q '^JOIN b ' "$RUN.serve.log"; do sleep 0.2; done
cat "$RUN.serve.log"
URL=$(sed -n 's/^JOIN b concord join //p' "$RUN.serve.log" | sed "s|//[^/:]*:|//127.0.0.1:|")

"$CONCORD" release x
"$CONCORD" add c | tee "$RUN.add.log"
check "add prints a JOIN line for the running server" grep -q '^JOIN c concord join http' "$RUN.add.log"
URL_C=$(sed -n 's/^JOIN c concord join //p' "$RUN.add.log" | sed "s|//[^/:]*:|//127.0.0.1:|")
check "adding a member again is refused" sh -c "! '$CONCORD' add c"

cd "$RUN-elsewhere"
"$CONCORD" join "$URL"
B=$RUN-elsewhere/$(basename "$RUN")-b
check "join made a copy for b, with the base's files" sh -c "test -f '$B/.concord/remote.json' && grep -q 'return 1' '$B/m.py'"
check "join is idempotent" "$CONCORD" join "$URL"
"$CONCORD" join "$URL_C"
C=$RUN-elsewhere/$(basename "$RUN")-c
check "the added agent joined with its new token" test -f "$C/.concord/remote.json"

CHECK="'$CONCORD' show \"\$COMBINED\" m.py | grep -q 'return 10 + 5' && '$CONCORD' show \"\$COMBINED\" n.py >/dev/null"
# a renames f's result first; b's edit builds on it, so b must receive a's change via the server.
( cd "$RUN-a" && "$CONCORD" plan a f - -- "f returns 10" && "$CONCORD" claim f &&
  sed -i.bak 's/return 1/return 10/' m.py && rm m.py.bak &&
  "$CONCORD" done f "f() -> int; CHANGED: returns 10" && finish_or_review "$CHECK" a ) > "$RUN.a.log" 2>&1 &
( cd "$B" && sleep 1 && "$CONCORD" plan b f - -- "f returns 15, building on a" && "$CONCORD" claim f &&
  grep -q 'return 10' m.py &&
  sed -i.bak 's/return 10/return 10 + 5/' m.py && rm m.py.bak &&
  "$CONCORD" done f "f() -> int; CHANGED: returns 15" && finish_or_review "$CHECK" b ) > "$RUN.b.log" 2>&1 &
# c plans last and uses f: its finish waits until a and b are done with f, then reports their
# changes (exit 10); c adapts (nothing to do here) and finishes again.
( cd "$C" && sleep 2 && "$CONCORD" plan c - f -- "g returns 2" && printf 'def g():\n    return 2\n' > n.py &&
  { finish_or_review "$CHECK" c || { [ $? -eq 10 ] && finish_or_review "$CHECK" c; }; } ) > "$RUN.c.log" 2>&1 &
wait %2 || { cat "$RUN.a.log"; echo "agent a failed"; exit 1; }
wait %3 || { cat "$RUN.b.log"; echo "agent b failed"; exit 1; }
wait %4 || { cat "$RUN.c.log"; echo "agent c failed"; exit 1; }
cat "$RUN.a.log" "$RUN.b.log" "$RUN.c.log"

check "b got a's change from the server before editing" grep -q "MERGED" "$RUN.b.log"
ZERO=0000000000000000000000000000000000000000000000000000000000000000
check "b's token cannot act for a" sh -c "curl -s -X POST --data '{\"Claim\":{\"agent\":\"a\",\"func\":\"f\"}}' '$URL/api/command' | grep -q 'this token belongs to b'"
# One object in the upload format (64-hex id, 8-byte big-endian length, bytes): "x" sent as $ZERO.
printf '%s\000\000\000\000\000\000\000\001x' "$ZERO" > "$RUN.bad-object"
check "an object that does not hash to its id is refused" test "$(curl -s -o /dev/null -w '%{http_code}' -X POST --data-binary @"$RUN.bad-object" "$URL/api/objects")" = 400
check "a done naming a change the server lacks is refused" sh -c "curl -s -X POST --data '{\"Done\":{\"agent\":\"b\",\"func\":\"f\",\"change\":\"$ZERO\",\"note\":\"x\"}}' '$URL/api/command' | grep -q 'not complete on the server'"
BAD=$(echo "$URL" | sed 's|/t/[0-9a-f]*|/t/0000|')
check "unknown token refused" test "$(curl -s -o /dev/null -w '%{http_code}' "$BAD/api/board")" = 403
check "status works from the remote clone" sh -c "cd '$B' && '$CONCORD' status | grep -q finished"
check "the last finisher reviewed and approved" grep -q "^APPROVED by" "$RUN.a.log" "$RUN.b.log" "$RUN.c.log"

cd "$RUN"
check "the project folder has both agents' work" grep -q 'return 10 + 5' m.py
check "the project folder has the added agent's work" grep -q 'return 2' n.py
check "git untouched: same HEAD, still on main" sh -c "[ \"\$(git rev-parse HEAD)\" = $GIT_HEAD ] && [ \"\$(git branch --show-current)\" = main ]"
echo "OK agents on another machine land through concord serve"
