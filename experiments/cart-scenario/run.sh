#!/usr/bin/env bash
# Reproduce the cart scenario end to end with the Rust binary, using scripted agents.
#
# Starts from a plain git repo, like a user's project: `concord init` snapshots the folder as the
# base and creates the agent workspaces; git stays the user's own tool and concord never touches it. Each agent is a bash function that follows the concord protocol and makes the same
# edits the Haiku agents made in the recorded v2 run (evidence/). It proves the binary enforces the
# protocol (barrier, queue order, merge on claim, blocking finish, exit 10) and the landing:
# the round's last finisher reviews the combined result (exit 12; scripted: the four tests) and
# approves it; the result is pulled into the folder as uncommitted edits. It does not measure LLM agent behavior; that is what evidence/ records.
#
# usage: experiments/cart-scenario/run.sh [run-dir]     (default: target/cart-run, gitignored)
# needs: mergiraf, python3.12 with pytest (override with PYTHON=...)
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
RUN=${1:-$ROOT/target/cart-run}
PYTHON=${PYTHON:-python3.12}
CONCORD=$ROOT/target/debug/concord
export PYTHONDONTWRITEBYTECODE=1
TEST=(env PYTHONDONTWRITEBYTECODE=1 "$PYTHON" -m pytest -q -p no:cacheprovider -p no:asyncio)

source "$ROOT/experiments/review.sh"

(cd "$ROOT" && cargo build --quiet)
rm -rf "$RUN"
mkdir -p "$RUN/repo"
cp "$HERE"/base/*.py "$RUN/repo/"
cd "$RUN/repo"
git init --quiet
git add .
git -c user.name=cart -c user.email=cart@example.invalid commit --quiet -m "base: cart.py"
GIT_HEAD=$(git rev-parse HEAD)

# --- helpers used by the scripted agents ------------------------------------------------------
replace() { # file old new: replace exactly one occurrence
  "$PYTHON" -I -c 'import sys
p, old, new = sys.argv[1:]
s = open(p).read()
if s.count(old) != 1: sys.exit(f"{p}: expected one {old!r}")
open(p, "w").write(s.replace(old, new))' "$@"
}
run_tests() { "${TEST[@]}" >/dev/null; }
# The scripted reviewer's check: the four tests on the combined change.
combined_tests() {
  local d; d=$(mktemp -d "$RUN/review.XXXX")
  for f in cart.py test_cart.py; do "$CONCORD" show "$COMBINED" "$f" > "$d/$f"; done
  (cd "$d" && "${TEST[@]}")
}

# --- agents -----------------------------------------------------------------------------------
agent_a() { # rename total -> subtotal
  cd "$RUN/repo-agent-a"
  "$CONCORD" claim total
  replace cart.py 'def total(items):' 'def subtotal(items):'
  printf '\n\ndef test_subtotal():\n    assert cart.subtotal(ITEMS) == 25\n' >> test_cart.py
  run_tests
  "$CONCORD" done total "subtotal(items) -> int: sum of price * qty; CHANGED: renamed from total"
  finish_or_review combined_tests agent-a
}

agent_b() { # add discount_pct; claim rebases it onto A's rename first
  cd "$RUN/repo-agent-b"
  "$CONCORD" claim total
  replace cart.py 'def subtotal(items):
    """Sum of price * qty for every item."""
    return sum(i["price"] * i["qty"] for i in items)' 'def subtotal(items, discount_pct=0):
    """Sum of price * qty over items, less discount_pct percent (0-100)."""
    gross = sum(i["price"] * i["qty"] for i in items)
    return gross * (1 - discount_pct / 100)'
  printf '\n\ndef test_subtotal_with_discount():\n    assert cart.subtotal(ITEMS, discount_pct=20) == 20\n' >> test_cart.py
  run_tests
  "$CONCORD" done total "subtotal(items, discount_pct=0) -> float; CHANGED: now always float, format before printing"
  finish_or_review combined_tests agent-b
}

agent_c() { # add receipt(); codes against the stale total() until finish says otherwise
  cd "$RUN/repo-agent-c"
  add_receipt 'return f"{item_count(items)} items, total {total(items)}"'
  run_tests
  local out code=0
  out=$("$CONCORD" finish) || code=$?
  echo "$out"
  [ "$code" -eq 10 ] || { echo "agent-c expected exit 10 from finish, got $code"; return 1; }
  # finish already merged the new contracts in. Stand-in for an agent resolving the conflict by
  # hand: take agent-b's files (its contract change), re-apply receipt adapted to the notes.
  local b_change; b_change=$(sed -n 's/.* by agent-b at \([0-9a-f]*\):.*/\1/p' <<<"$out" | tail -1)
  for f in cart.py test_cart.py; do "$CONCORD" show "$b_change" "$f" > "$f"; done
  add_receipt 'amount = subtotal(items)
    shown = int(amount) if amount == int(amount) else amount
    return f"{item_count(items)} items, total {shown}"'
  run_tests
  finish_or_review combined_tests agent-c
}

add_receipt() { # body
  printf '\n\ndef receipt(items):\n    """One-line summary: item count and cart total."""\n    %s\n' "$1" >> cart.py
  printf '\n\ndef test_receipt():\n    assert cart.receipt(ITEMS) == "3 items, total 25"\n' >> test_cart.py
}

# --- run --------------------------------------------------------------------------------------
"$CONCORD" init agent-a agent-b agent-c
# Queue order is plan order. Scripted agents cannot adapt to another order (B's edit assumes A's
# rename), so the replay plans in the order the recorded run did (evidence/v2-board-log.txt).
"$CONCORD" plan agent-a total - -- "total is renamed to subtotal with unchanged behavior; nothing calls total"
"$CONCORD" plan agent-b total - -- "subtotal(items, discount_pct=0) returns gross * (1 - discount_pct / 100); subtotal(ITEMS, discount_pct=20) == 20"
"$CONCORD" plan agent-c - total,item_count -- 'receipt(items) returns "<count> items, total <total>"; receipt(ITEMS) == "3 items, total 25"'
pids=()
for a in a b c; do
  ( "agent_$a" ) > "$RUN/agent-$a.out" 2>&1 &
  pids+=($!)
done
status=0
for i in 0 1 2; do
  wait "${pids[$i]}" || { status=1; echo "agent ${i} (a=0 b=1 c=2) failed"; }
done
for a in a b c; do echo "--- agent-$a"; cat "$RUN/agent-$a.out"; done
[ "$status" -eq 0 ] || exit 1

echo "--- board log"
"$CONCORD" log | tee "$RUN/board-log.txt"
grep -q "^REVIEW REQUIRED agent-" "$RUN"/agent-*.out || { echo "nobody was asked to review"; exit 1; }
grep -q "LANDED agent-a,agent-b,agent-c" "$RUN/board-log.txt" || { echo "the round did not land"; exit 1; }
for a in a b c; do [ ! -e "$RUN/repo-agent-$a" ] || { echo "workspace agent-$a not removed"; exit 1; }; done
diff "$HERE/evidence/v2-final-cart.py" cart.py
diff "$HERE/evidence/v2-final-test_cart.py" test_cart.py
[ "$(git rev-parse HEAD)" = "$GIT_HEAD" ] || { echo "git HEAD moved"; exit 1; }
[ "$(git status --porcelain | sort | tr '\n' ' ')" = " M cart.py  M test_cart.py " ] ||
  { echo "the landed round is not uncommitted edits in the folder"; git status --porcelain; exit 1; }
echo "OK combined code matches evidence/v2-final-*.py"
