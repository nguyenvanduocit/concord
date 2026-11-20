#!/usr/bin/env bash
# Landing in a folder that is a git repo with uncommitted work, seen from the git user's side.
#
# A git repo on `main` with uncommitted work (a modified tracked file and an untracked file); the
# user keeps editing during the round. Two scripted agents change different functions; the last to
# finish reviews the combined result.
#   approved: the landed files are in the folder as uncommitted edits, the user's edits (from
#             before and during the round) survive, .git is untouched (same files, same HEAD,
#             same log).
#   rejected: the folder is exactly as it was (every file, .git included).
#
# usage: experiments/landing/run.sh [run-dir]     (default: target/landing, gitignored)
# needs: git, mergiraf
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
RUN=${1:-$ROOT/target/landing}
CONCORD=$ROOT/target/debug/concord
source "$ROOT/experiments/review.sh"
(cd "$ROOT" && cargo build --quiet)

check() { # description, command...
  local what=$1; shift
  if "$@"; then echo "ok   $what"; else echo "FAIL $what"; exit 1; fi
}

# Checksum of every file under the folder (or under $1), .concord left out. No git command runs
# between two of these: git status alone rewrites .git/index.
sums() { (cd "${1:-.}" && find . -path ./.concord -prune -o -type f -print0 | sort -z | xargs -0 shasum) | shasum; }

# Fresh repo: main has m.py; the user has uncommitted work in README.md and notes.txt.
setup() {
  rm -rf "$RUN" "$RUN"-*
  mkdir -p "$RUN" && cd "$RUN"
  git init --quiet -b main
  printf 'def f():\n    return 1\n\n\ndef g():\n    return 2\n' > m.py
  echo "readme" > README.md
  git add . && git -c user.name=t -c user.email=t@example.invalid commit --quiet -m "init"
  echo "readme, edited" > README.md
  echo "private wip" > notes.txt
  GIT_HEAD=$(git rev-parse HEAD)
  GIT_LOG=$(git log --all --format='%H %s')
}

agents() { # agent a sets f to $1, agent b sets g to $2; the reviewer runs CHECK
  ( cd "$RUN-a" && "$CONCORD" plan a f - -- "f returns $1" && "$CONCORD" claim f >/dev/null &&
    sed -i.bak "s/return 1/return $1/" m.py && rm m.py.bak &&
    "$CONCORD" done f "f() -> int; CHANGED: returns $1" >/dev/null && finish_or_review "$CHECK" a ) &
  ( cd "$RUN-b" && "$CONCORD" plan b g - -- "g returns $2" && "$CONCORD" claim g >/dev/null &&
    sed -i.bak "s/return 2/return $2/" m.py && rm m.py.bak &&
    "$CONCORD" done g "g() -> int; CHANGED: returns $2" >/dev/null && finish_or_review "$CHECK" b ) &
  wait
}

git_untouched() {
  [ "$(git rev-parse HEAD)" = "$GIT_HEAD" ] && [ "$(git branch --show-current)" = main ] &&
    [ "$(git log --all --format='%H %s')" = "$GIT_LOG" ]
}

echo "=== approved: the round is pulled into the folder"
setup
GIT_SUM=$(sums .git)
CHECK="'$CONCORD' show \"\$COMBINED\" m.py | grep -q 'return 10'"
"$CONCORD" init a b >/dev/null
echo "edited during the round" >> notes.txt # the user keeps working while the agents do
agents 10 20
check ".git untouched (every file)" test "$(sums .git)" = "$GIT_SUM"
check "HEAD, branch and log unchanged" git_untouched
check "the folder has both agents' changes" sh -c "grep -q 'return 10' m.py && grep -q 'return 20' m.py"
check "they are uncommitted edits" test "$(git status --porcelain | sort | tr '\n' ' ')" = " M README.md  M m.py ?? notes.txt "
check "the user's edit from before the round survived" test "$(cat README.md)" = "readme, edited"
check "the user's edits during the round survived" test "$(cat notes.txt)" = "$(printf 'private wip\nedited during the round')"
check "the landing is in the ledger" grep -q '"landed"' .concord/ledger.jsonl
check "agent workspaces removed" sh -c "[ ! -e '$RUN-a' ] && [ ! -e '$RUN-b' ]"
check "the review checkout is gone" test ! -e "$RUN-concord-gate"

echo "=== rejected: the folder stays exactly as it was"
setup
CHECK=false
"$CONCORD" init a b >/dev/null
BEFORE=$(sums)
agents 10 20
check "every file in the folder unchanged, .git included" test "$(sums)" = "$BEFORE"
check "the rejection is on the board" sh -c "'$CONCORD' log | grep -q ' REJECT '"
check "HEAD, branch and log unchanged" git_untouched
check "agent workspaces kept for fixing" sh -c "[ -d '$RUN-a' ] && [ -d '$RUN-b' ]"
check "the review checkout is gone" test ! -e "$RUN-concord-gate"
echo "OK a landing is a pull into the folder; git is the user's and stays untouched"
