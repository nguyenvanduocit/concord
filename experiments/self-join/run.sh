#!/usr/bin/env bash
# Agents join on their own, in a folder others keep editing: no `concord init`.
#
# A git repo on `main` whose folder is busy: a session that does not use concord has uncommitted
# work there and keeps appending to a file during the whole run. Agents a and b each start with
# `concord plan` from that folder (their first command creates the board / joins the team and
# their own workspace), take turns on the same function, finish; the last finisher reviews the
# combined result (a slow check) and approves it: it is pulled into the folder. The next round's
# only agent gets a name of its own and reviews and lands its own round. The busy session must
# lose nothing: its draft (part of the base: the base is the folder as it was at the first plan)
# and every line it appended, also while the review was checking. git is never touched.
#
# usage: experiments/self-join/run.sh [run-dir]     (default: target/self-join, gitignored)
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
RUN=${1:-$ROOT/target/self-join}
CONCORD=$ROOT/target/debug/concord
source "$ROOT/experiments/review.sh"
(cd "$ROOT" && cargo build --quiet)

check() { # description, command...
  local what=$1; shift
  if "$@"; then echo "ok   $what"; else echo "FAIL $what"; exit 1; fi
}

rm -rf "$RUN" "$RUN"-* "$RUN".*
mkdir -p "$RUN"
cd "$RUN"
git init --quiet -b main
printf 'def f():\n    return 1\n' > m.py
printf 'notes\n' > notes.txt
git add . && git -c user.name=t -c user.email=t@example.invalid commit --quiet -m "init"
GIT_HEAD=$(git rev-parse HEAD)

# The busy session: an uncommitted edit, and a line appended every 0.2 s until told to stop.
printf 'draft by the other session\n' >> notes.txt
( i=0; while [ ! -e "$RUN.stop" ]; do i=$((i+1)); echo "line $i" >> busy.log; sleep 0.2; done
  echo "$i" > "$RUN.count" ) &
BUSY=$!
trap 'touch "$RUN.stop"; kill $BUSY 2>/dev/null || true' EXIT

check "no board before the first plan" sh -c "! '$CONCORD' status >/dev/null 2>&1"
"$CONCORD" plan a f - -- "f returns 10" | tee "$RUN.a.plan"
check "a's first plan made its workspace" grep -q "^WORKSPACE a $RUN-a" "$RUN.a.plan"
"$CONCORD" plan b f - -- "f returns 15, building on a" | tee "$RUN.b.plan"
check "b joined the running team" grep -q "^JOINED b" "$RUN.b.plan"
check "planning again does not join again" sh -c "! '$CONCORD' plan b f - -- 'f returns 15, building on a' | grep -q JOINED"
check "a plan without done-criteria is refused" sh -c "'$CONCORD' plan b f - 2>&1 | grep -q 'needs your done-criteria'"

# A slow check, so the busy session writes while the review is checking.
CHECK="sleep 2 && '$CONCORD' show \"\$COMBINED\" m.py | grep -q 'return 10 + 5'"

( cd "$RUN-a" && "$CONCORD" claim f && sed -i.bak 's/return 1/return 10/' m.py && rm m.py.bak &&
  "$CONCORD" done f "f() -> int; CHANGED: returns 10" && finish_or_review "$CHECK" a ) > "$RUN.a.log" 2>&1 &
( cd "$RUN-b" && "$CONCORD" claim f && grep -q 'return 10' m.py &&
  sed -i.bak 's/return 10/return 10 + 5/' m.py && rm m.py.bak &&
  "$CONCORD" done f "f() -> int; CHANGED: returns 15" && finish_or_review "$CHECK" b ) > "$RUN.b.log" 2>&1 &
wait %2 || { cat "$RUN.a.log"; echo "agent a failed"; exit 1; }
wait %3 || { cat "$RUN.b.log"; echo "agent b failed"; exit 1; }
cat "$RUN.a.log" "$RUN.b.log" | tee "$RUN.gate"
check "the last finisher was asked to review" grep -q "^REVIEW REQUIRED" "$RUN.gate"
check "its approval landed the round" grep -q "^LANDED " "$RUN.gate"
check "the round ended for a and b" grep -q "^ROUND a, b landed" "$RUN.gate"
check "the folder has both agents' work" grep -q 'return 10 + 5' m.py
check "the gate workspace is gone" test ! -e "$RUN-concord-gate"

check "worker commands in the main folder point to the workspace" \
  sh -c "'$CONCORD' claim f 2>&1 | grep -q 'run this inside your workspace'"

# The next round: an agent with no name gets one and starts from the landed code; its finish, the
# round's last, asks it to review the round (its own work: no other session) and it lands it.
"$CONCORD" plan - f -- "g() returns 2" | tee "$RUN.c.plan"
check "an agent without a name gets a free one" grep -q "^WORKSPACE agent-1 $RUN-agent-1" "$RUN.c.plan"
check "it starts from the landed code" grep -q 'return 10 + 5' "$RUN-agent-1/m.py"
( cd "$RUN-agent-1" && printf 'def g():\n    return 2\n' > n.py &&
  finish_or_review "'$CONCORD' show \"\$COMBINED\" n.py | grep -q 'return 2'" agent-1 ) | tee "$RUN.c.finish"
check "its finish asked it to review its round" grep -q "^REVIEW REQUIRED agent-1" "$RUN.c.finish"
check "the review is marked as self-reviewed" grep -q "^SELF-REVIEWED agent-1" "$RUN.c.finish"
check "its approval landed the round in the folder" grep -q 'return 2' n.py

touch "$RUN.stop"; wait $BUSY || true
check "the busy session's uncommitted edit survived" grep -q 'draft by the other session' notes.txt
check "every line it appended survived" test "$(wc -l < busy.log | tr -d ' ')" = "$(cat "$RUN.count")"
check "git untouched: same HEAD, still on main" sh -c "[ \"\$(git rev-parse HEAD)\" = $GIT_HEAD ] && [ \"\$(git branch --show-current)\" = main ]"
echo "OK agents join on their own in a busy folder, nothing of the other session is lost"
