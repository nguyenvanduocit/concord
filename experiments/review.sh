# Sourced by the experiments: the scripted agents' finish, and their review.
#
# Scripted agents have no coding-agent session of their own. When a script runs inside one (a
# Claude Code or Codex session), concord would record that session for every scripted agent and
# could continue it: clear the variables it reads.
unset CLAUDE_CODE_SESSION_ID CODEX_THREAD_ID CODEX_SESSION_ID

# finish; exit 12 means this agent reviews the round. Stand-in for an agent checking the work
# against the criteria: CHECK (shell, with $COMBINED = the combined change, readable with
# `concord show "$COMBINED" <file>`) passing approves (`concord land`), failing rejects AUTHOR.
finish_or_review() { # check author
  local out code=0
  out=$("$CONCORD" finish) || code=$?
  printf '%s\n' "$out"
  [ "$code" -eq 12 ] || return "$code"
  COMBINED=$(sed -n 's/^COMBINED \([0-9a-f]*\).*/\1/p' <<<"$out")
  if (eval "$1"); then
    "$CONCORD" land "scripted check passed: $1"
  else
    "$CONCORD" reject "$2" "scripted check failed: $1"
  fi
}
