#!/usr/bin/env bash
# Gate check for CooperBench cases: the combined result must pass every agent's feature tests.
#
# usage (run by bench/run.py in the combined checkout): check_cooperbench.sh <case-dir> <base-commit>
# For each feature's tests: a fresh clone at the base, then the task's own run_tests.sh
# (`run_tests.sh <repo> <tests.patch> <change.patch>`), with the combined change (base..HEAD).
# Control first: the dataset's own resolved PR (oracle.diff) must pass the same tests here, else
# the environment is broken and the case says so (exit 3, `ENV BROKEN`) instead of blaming the
# merge. Control results are cached per task and feature. Runs on the host: every cache and
# build dir goes under $BENCH_CACHE (external drive).
set -euo pipefail
CASE=$1
BASE=$2
: "${BENCH_CACHE:?set by bench/run.py}"
export TMPDIR=$BENCH_CACHE/tmp XDG_CACHE_HOME=$BENCH_CACHE/xdg
export UV_CACHE_DIR=$BENCH_CACHE/uv PIP_CACHE_DIR=$BENCH_CACHE/pip
export GOPATH=$BENCH_CACHE/go GOCACHE=$BENCH_CACHE/go-build GOFLAGS=-modcacherw
export npm_config_cache=$BENCH_CACHE/npm npm_config_store_dir=$BENCH_CACHE/pnpm-store
export CARGO_HOME=$BENCH_CACHE/cargo-home CARGO_BUILD_BUILD_DIR=$BENCH_CACHE/cargo-build
mkdir -p "$TMPDIR" "$XDG_CACHE_HOME" "$BENCH_CACHE/control"
WORK=$(mktemp -d "$TMPDIR/coop.XXXXXX")
trap 'rm -rf "$WORK"' EXIT
field() { python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["check"]["cooperbench"][sys.argv[2]])' "$CASE/case.json" "$1"; }
RUNNER=$CASE/$(field runner)
ORACLE=$CASE/$(field oracle)
TASK=$(field task | tr / -)
SOURCE=$PWD
git diff --binary "$BASE" HEAD > "$WORK/combined.patch"

# run_tests <tests.patch> <change.patch> <log>: the task's runner on a fresh clone at the base.
# Some runners only operate inside a folder named agent_workspace.
run_tests() {
  local repo=$WORK/agent_workspace/repo
  rm -rf "$repo" && mkdir -p "$WORK/agent_workspace"
  git clone --quiet --shared --no-checkout "$SOURCE" "$repo"
  git -C "$repo" checkout --quiet "$BASE"
  bash "$RUNNER" "$repo" "$1" "$2" > "$3" 2>&1
}

for tests in "$CASE"/tests/feature*.patch; do
  name=$(basename "$tests" .patch)
  control=$BENCH_CACHE/control/$TASK-$name
  if [ ! -e "$control.ok" ] && [ ! -e "$control.broken" ]; then
    if run_tests "$tests" "$ORACLE" "$WORK/control.log"; then touch "$control.ok"
    else cp "$WORK/control.log" "$control.broken"; fi
  fi
  if [ -e "$control.broken" ]; then
    echo "ENV BROKEN $name: the dataset's own resolution fails these tests here ($control.broken)"
    exit 3
  fi
  if ! run_tests "$tests" "$WORK/combined.patch" "$WORK/$name.log"; then
    echo "TESTS FAILED $name"
    tail -15 "$WORK/$name.log"
    exit 1
  fi
done
