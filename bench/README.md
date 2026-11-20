# bench: every public benchmark as one test

One case format and one runner for the community benchmarks on merging and on parallel coding
agents, so concord is checked on cases nobody here thought of.

```bash
python3 -I bench/import.py /Volumes/Data/src-untrusted/benchmarks /Volumes/Data/concord-bench/cases
python3 -I bench/run.py /Volumes/Data/concord-bench/cases /Volumes/Data/concord-bench/out \
    [--source S] [--sample N] [--jobs J]
```

Cases and results live outside this repo (the runner refuses an out dir inside it): a case's
build would otherwise find concord's own `Cargo.toml` workspace above it (cargo walks up) and fail.

The runner puts each case through concord as a user would: a git repo at the case's base on
`main`, `concord init <agents>`, each agent applies its change in its own workspace and runs
`plan`/`finish`; the last finisher is asked to review the combined result (exit 12), runs the
case's check in the combined checkout in place of reading it, and answers `land` or `reject`.
Verdicts: `pass`, `conflict` (combining left conflicts), `wrong` (merged, but the check failed), `error` (harness or concord failed), `env` (control: the
dataset's own resolution fails the same check here, so the environment is at fault, not concord). Merge cases
also get a plain `git merge-file` verdict on the same three versions, for comparison. Results go to
`<out>/results.jsonl` (an interrupted run resumes) and `<out>/summary.md`.

| Source | Cases | What a case is | Check |
|---|---|---|---|
| [ConGra](https://github.com/HKU-System-Security-Lab/ConGra) | 11,234 | one conflicting file: base, two sides, the developer's resolution (Python, Java, C/C++), labelled text / syntax / function conflict | merged file = resolution |
| [ConflictBench](https://github.com/UBOWENVT/ConflictBench) | 159 of 180 | a Java merge scenario: base, left, right, merged child (21 lack one of the four trees) | merged files = child |
| [CooperBench](https://github.com/cooperbench/CooperBench) | 652 | two features of one real PR, each an agent's gold patch (12 repos, 4 languages) | both features' hidden tests pass (the task's own `run_tests.sh`, run on the host) |

Not included: Merge-Bench (conflict snippets without the base version, so no three-way merge can
run), AgenticFlict (commit ids of 59k repos, nothing to merge without cloning them), "Passes
Alone, Fails Together" and BulkPR-Bench (no public data found).

Downloads live in `/Volumes/Data/src-untrusted/benchmarks/` and are only read: the converter
copies regular files (never symlinks). CooperBench's tests run on the host (the 37 Docker images,
~80 GB, do not fit the internal disk): the 12 repos' code and dependencies run unsandboxed, with
every cache and build dir under `<out>/cache` on the external drive.
