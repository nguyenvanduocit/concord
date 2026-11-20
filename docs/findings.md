# Findings: parallel agents editing one file

Measured 2026-11-18 on macOS (jj 0.47.1, GitButler `but` 0.23.4, Mergiraf 0.20.0, Python 3.12).
Agents were Claude Haiku subagents. One run per setup, a scenario designed to collide — these
results show failure modes, not frequencies. Scenario and evidence: `experiments/cart-scenario/`.

Entries are dated and record what was measured then. Those before 2026-11-20 measured the
jj-based version of concord; "concord's own store replaces jj and git (2026-11-20)" covers the
current one.

## The scenario

`cart.py` has `total(items)` and `item_count(items)`. Three agents start from the same base, none
told about the others:

| Agent | Task | Collides with |
|---|---|---|
| A | rename `total` → `subtotal` | B (same function), C (C calls it) |
| B | add `discount_pct=0` to `total` | A (same function); its formula makes the result a float |
| C | add `receipt()` calling `total`, test expects `"3 items, total 25"` | A (name), B (float prints `25.0`) |

A correct combined result passes 4 tests. The two semantic bugs are the point:
- rename → caller still calls `total` (build-layer conflict, merges cleanly as text)
- float → `receipt` prints `25.0` (semantic-layer conflict, no textual conflict at all)

## Results

| Setup | Agents committed | Where the conflict surfaced | Auto-resolved by tooling | Combined tests after naive merge |
|---|---|---|---|---|
| plain `git worktree` | 3/3 | merge stops at B; C cannot merge until a human resolves | nothing | same code as the Mergiraf row |
| git + Mergiraf (merge driver) | (same commits) | stops at B | `cart.py` rename + body change → `subtotal(items, discount_pct=0)` | 2/4 |
| jj workspaces | 3/3 | stored as a 3-sided conflict inside the merge commit, nothing blocks | nothing | B+C alone: fails on `25.0` |
| jj + Mergiraf | (same commits) | jj merge tools take 2 sides only → pairwise merges | `cart.py`, same as above | A+B already 1/3 failing |
| GitButler, one shared directory | **0/3** | live, while editing: hunks of different agents fuse, cannot be split by ID | — | 2/4, B's code never landed |
| GitButler + `but worktree new` | 3/3 | rebase completes, commits marked `{conflicted}`; refuses to land a conflicted commit | nothing | 2/4 — landed B with failing tests |
| jj + coord v1 (claims + DONE notes) | 3/3 | B stacked on A, C stacked on A | — | 3/4: rename caught, float missed |
| **jj + coord v2 + gate** | 3/3 | serialized A → B → C on `total`; C resolved its own rebase conflict | — | **4/4 PASS**, first gate run |

## What each layer is worth

1. **Combined-test gate before landing** is the only thing that detected every semantic bug.
   Every setup without it landed broken code.
2. **Coordination** prevents semantic bugs instead of only detecting them.
   - v1 missed the float because C finished before B announced it (race).
   - v2 fixed that with a `plan` barrier (all intents known before work) and a blocking `finish`
     that returns exit 10 with unseen contract changes. C rebased, switched to `subtotal`, and
     formatted the float — log: `FINISH agent-c ... new_contracts=['total#1', 'total#2']`.
3. **Structural merge (Mergiraf)** resolves conflicts inside one function. It does not resolve two
   top-level functions appended at the same spot (`test_cart.py`), in git or jj.
4. **jj** keeps merges and rebases from blocking and gives `jj undo`; history stays clean.
   The jj vs git difference was merge flow, not correctness.

## Known gaps after v2

- B's contract note said "returns int|float … a number" instead of "now always float". C caught the
  float by reading code after the rebase, not from the note. Free-text notes are too weak.
- The claim key is the original name (`total`); B's `done subtotal` was rejected. Keys must follow renames.
- `plan` was not idempotent (an agent planned twice → duplicate queue slot → B blocked). Fixed in the
  prototype; the live board needed one manual repair (logged as `REPAIR`).
- v2 serializes agents that touch the same function, so this scenario shows correctness, not speedup.
  Speedup needs a scenario where most tasks are independent.

## Tool notes

- jj 0.47.1: in a colocated repo `jj workspace add` creates a real Git worktree; delete workspace
  folders only after `jj workspace forget`, or run `git worktree prune`. `description("x")` in a
  revset is an exact match. Paths in `jj file show` need `root:"file"`.
- GitButler: CLI is a symlink into `~/Applications/GitButler.app`; `but worktree new` puts worktrees
  under `~/.gitbutler-worktrees/`. Conflict display (ours/base/theirs per commit) is the clearest of the tools.
- Mergiraf as git driver needs `.git/info/attributes` (or `.gitattributes`) `*.py merge=mergiraf`.
- Pijul 0.15.0 did not build: `--locked` fails on rustc 1.100 (`regex-automata`); not tested.

## Dogfood: concord developing concord (2026-11-18)

Three Claude Sonnet subagents, one jj workspace each, briefed only with the `concord` skill, their
task and their scope. Tasks: `finish` snapshots the workspace (agent-a), gate fails on an empty
finished change (agent-b), `concord status` (agent-c).

- agent-a and agent-c both planned `modifies=['run']`; agent-c waited, claimed after agent-a's
  `done`, got `LATEST plxyvnurstqv` and rebased before editing.
- agent-a and agent-b both added methods to `Jj` in `vcs.rs`; jj merged them without a conflict.
- Gate: tips `zxkkxnux` + `rxwmwswn`, clippy clean, 25 tests passed, PASS on the first run.
- Skill gaps found: agent-a first planned `modifies=[]` for a function whose body it changed, and
  edited before `claim`. No harm here (no `LATEST`), but either one lets a parallel edit go
  unnoticed. The skill now states both rules explicitly.
- Docs are not functions: all three would have edited `docs/design.md` in parallel; the
  orchestrator kept docs to itself and edited them after the gate.

## concord on an existing git repo (2026-11-18)

A git repo on `main` with uncommitted work, two scripted agents, a passing gate. Before landing was
designed in, a git user saw: detached HEAD, `main` not moved, untracked `notes.txt` committed into
the agents' base (jj tracks every new file), an empty dangling `gate: combined` commit (the gate
created it before merging), the git index out of step with HEAD (`Changes to be committed`).
`git log --all` also lists `refs/jj/keep/*` commits: jj's guard against git gc in colocated repos,
invisible in `git log`/`git log main`.

Fixes, checked by `experiments/git-landing/run.sh` (since replaced by `experiments/landing/run.sh`,
see the 2026-11-20 entry) on the Air and macmini-kiot: agents start from
the last commit, the main workspace tracks files like git (`snapshot.auto-track = none()`), the
gate tests in a scratch child and lands like a pull (branch fast-forwarded, HEAD attached, working
copy on top) or restores everything on FAIL. jj drops an empty working copy when `@` leaves it, so
restoring needs its parent too (seen in the cart scenario, which has no uncommitted work).

## Public benchmarks: ConflictBench, merge tool A/B (2026-11-18)

`bench/` runs every public merge benchmark through concord (README there). ConflictBench, 159 real
Java merge scenarios, check = merged files equal the developer's resolution, text-exact:

| Gate merge tools | pass | conflict | wrong |
|---|---|---|---|
| plain `git merge-file` (baseline; 2 cases error) | 0 | 157 | 0 |
| Mergiraf | 27 | 106 | 26 |
| Mergiraf, then Weave (`weave-driver`) | 37 | 85 | 37 |

Weave turned 21 conflicts into merges: 10 equal the developer's, 11 differ. Of the 11 (replayed
with a logging wrapper around `weave-driver`): 3 equivalent (whitespace, import order), 3 harmless
extras (both sides kept), 5 broken: `catch (RuntimeException | RuntimeException ex)` and a doubled
`@Generated` (do not compile), a dropped import, a dropped new parameter (kafka), 274 lines off
(nanohttpd). A conflict costs one manual resolution; a wrong merge lands silently unless the tests
catch it. Rule set before the run: keep Weave only if pass rises without wrong rising. It failed,
so the gate uses Mergiraf alone.

- jj hands a merge tool the sides with the hunks jj resolved itself already applied, so a tool
  sees less conflict inside the gate than on the raw files (Weave exits 1 standalone, 0 in jj).
- With exit 1 in `merge-conflict-exit-codes`, jj reparses the output for markers; a hunk the tool
  marks with an empty ours and base simplifies to theirs, so no markers remain.
- "wrong" is text inequality: some wrong merges are fine code. Use it to compare tools, not as a
  count of bugs.

## Agent review instead of tests (2026-11-19)

Question: can the landing gate hand the verdict to an agent instead of running the project's
tests? (Answered yes: `design.md`, Landing.) `experiments/agent-review/`: the cart scenario's combined code in
six variants, a reviewer with no tools given the done-criteria, the contract notes and the code,
answering `LAND` or `REJECT <agent>`; claude haiku and sonnet, 5 runs per case, rule fixed before
each run, truth = the four tests plus hidden checks.

| Run | Reviewer reads criteria against | Main bugs rejected | Correct code landed | `:g` bug the tests miss rejected |
|---|---|---|---|---|
| 1 | combined code | 30/30 | sonnet 5/10, haiku 5/10 (one case was mislabeled, one criterion contradicted its task) | (case labeled correct) |
| 2 | combined code | 30/30 | sonnet 10/10, haiku 6/10 | sonnet 2/5, haiku 4/5 |
| 3 | each agent's own head | 30/30 | 20/20 | sonnet 0/5, haiku 2/5 |

- The bug the test gate exists for, the float with B's vague note, was rejected 30/30 across the
  runs, without running code.
- Criteria describe the agent's own change. Read against the combined code, a later declared
  change by another agent "breaks" them: haiku rejected correct code 3/5 for it (run 2). Read
  against each agent's own head, no false rejections (run 3).
- A bug no criterion names is found by chance (`:g` prints 1234567 as `1.23457e+06`): 0/5 to 5/5
  for the same code across runs. The tests find it 0/5. Scoped review behaves like a test of the
  criteria: as good as the criteria.
- Run 1's two "false" rejections were the reviewer being right: a real bug the four tests miss,
  and a criterion that contradicted its own task. A reviewer is only as good as the criteria.

## External evidence (2026 literature, checked 2026-11-20)

From a deep-research run: claims extracted from 22 sources, each checked by three independent
verifiers. A claim was kept only if at least two of the three upheld it. Almost all the
agent-era data comes from single 2026 arXiv preprints, not replicated: the numbers show failure
modes, not settled rates.

- **Free-text coordination fails** ([CooperBench, 2601.13295](https://arxiv.org/abs/2601.13295)):
  two agents splitting a pair of tasks succeed ~30% less often than one agent doing both (GPT-5
  315 → 183 over 600+ task pairs). Root causes in 50 failed traces: wrong beliefs about the other's plan 42, broken
  commitments 32, communication 26. Turning chat off made no significant difference. The authors
  recommend checkable shared state (pasted signatures, insertion-point contracts). Setup:
  separate containers, messages only, no shared workspace, so a board like concord's is untested.
- **A stale base causes conflicts that claims cannot**
  ([2610.04779](https://arxiv.org/abs/2610.04779)): in a sample of agent PR pairs, 47.3% of the
  conflicts happened even though the two PRs edited different files. All 48 that could be
  reconstructed merged cleanly on a shared base. concord merges `LATEST` at `claim` and unseen
  contract changes at `finish` for this reason.
- **A clean merge proves nothing** ([2609.25396](https://arxiv.org/abs/2609.25396)): when one
  agent changes an interface another uses, the combined code failed in 97% of constructed runs
  (only 1 of 834 mined, already-merged Django PR pairs). This supports a gate on the combined
  result.
- **Isolation beats a shared directory** ([2603.21489](https://arxiv.org/abs/2603.21489)):
  worktrees 63.3% vs soft isolation in one directory 55.5% vs one agent 57.2% (PaperBench). More
  agents can do worse (8 below 4 on Commit0-Lite). This agrees with the GitButler result above.
- **Structured merge trades one error for another** ([ASE 2019,
  peer-reviewed](https://www.se.cs.uni-saarland.de/publications/docs/CBS+19.pdf)): semistructured
  merge reports more false conflicts (36 vs 4), structured merge misses more real ones (39 vs 5).
  Generic tree-sitter tools such as Mergiraf lose no measurable accuracy against Java-only tools
  ([LastMerge, 2507.19687](https://arxiv.org/abs/2507.19687v1)). Either way, something has to
  check the combined code after the merge tool.
- **A prompt cannot regenerate its patch** ([2308.02828](https://arxiv.org/abs/2308.02828)): for
  47–76% of tasks, identical requests gave different test results, and temperature 0 does not
  make the output deterministic. A change is its commit; the prompt is only extra information.
- Refuted, do not cite: the 27.67% conflict rate for agent PRs (2604.03551), "notifying the
  second agent recovers 82% of failing runs" (2609.25396), the CAID gains of +25.6/+14.7, and
  "a scope gate de-overlaps 91.7% of collisions at a 65% makespan cost" (2610.04779).

## concord's own store replaces jj and git (2026-11-20)

concord now keeps its files itself (`design.md`, Store): BLAKE3 objects in `.concord/`, Mergiraf
on every file both sides changed, no jj fork, no git calls. The four end-to-end experiments pass
on it unchanged in what they check of the protocol:

- `cart-scenario`: the combined code is byte-identical to `evidence/v2-final-*.py`; agent-c's exit
  10 left Mergiraf conflict markers in `test_cart.py` (both appends at the file end), which the
  scripted agent settled; the landed round is two uncommitted edits (`cart.py`, `test_cart.py`)
  and git's HEAD did not move.
- `landing` (replaces `git-landing`): with uncommitted work in a git repo, an approved round
  arrives as uncommitted edits, the user's edits from before and during the round survive, and
  every file under `.git` has the same checksum before and after; a rejected round leaves every
  file of the folder as it was.
- `remote-agents`: a joined copy fetches the base and a's change by hash, uploads its own, a remote
  agent reviews and approves; the server refuses an object whose bytes do not hash to its id and a
  `done` naming a change it does not fully hold.
- `self-join`: the busy session's draft (part of the base) and every line it appended survive two
  rounds; git's HEAD did not move.

Found on the way: two finishes at once could both combine (each read the board before taking
`gate.lock`), and the second combine deleted the first one's review checkout under its reviewer.
The board is now read under the lock (`landing` reproduced it before the fix, 3/3 clean after).

Store speed, release build, 10 000 files of 3.9 KB (39 MB) on the external APFS drive
(`/Volumes/Data`), one run each:

| Operation | Time |
|---|---|
| first snapshot, every object new | 41.7 s (an earlier `concord init` of the same folder, snapshot plus workspace: 20.5 s) |
| snapshot, stat cache cold (no new objects) | 0.44 s |
| snapshot, stat cache warm | 0.12 s |
| commit with one changed file | 0.23 s |
| materialize into an empty folder | 13.6 s |

A profile (`sample`) of the first snapshot spends about 65% of its time in the `fsync` of each new
object, then `rename` and `open`. The steady state (`done`, `finish`: a few changed files) is
fast; creating a workspace or the review checkout writes every file and costs about 1.4 ms per file
on this drive, so a 10 000-file project pays ~14 s per agent workspace and per round's checkout.
Not measured: an internal SSD, or parallel writes.
