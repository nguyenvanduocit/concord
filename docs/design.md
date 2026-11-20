# concord design

Goal: parallel coding agents on one repo land correct combined code without a human resolving
conflicts. Evidence for every choice below is in `findings.md`.

## Stack

| Layer | Choice | Why |
|---|---|---|
| Isolation | one folder per agent, materialized from concord's own store | the protocol needs five file operations, nothing more (Store); no VCS to set up or to collide with the user's |
| Coordination | `concord` board (this project) | the only layer that prevented semantic bugs |
| Merge driver | Mergiraf, on every file both sides changed | resolves same-function conflicts git cannot; merges any text line-based where it has no grammar; Weave lost the ConflictBench A/B (findings) |
| Landing | combine + mechanical checks in concord, review by the team's own agents | tests are written by the same agents; a reviewer reading each agent's criteria decided every case the tests decide (findings) |

## Protocol

Any folder works, with or without git: concord keeps its files in its own store (Store, below)
and never reads or writes `.git`.

Board state: one JSON file (`state.json`) in the project's `.concord/` (the main folder's).
concord finds it from the current directory: the nearest folder with `.concord/workspace.json` is
the folder the command runs in; an agent workspace's `workspace.json` names the project's
`.concord` (`store`), so every workspace (or subdirectory) of a project reaches the same board,
and each project has its own board. Each
attempt to apply a command checks and mutates inside one exclusive `flock`; a blocking command
that must wait releases the lock and retries every 2 s. One call waits at most 100 s, then exits 11
(`WAITING for ...`: run the same command again): an agent's tool kills commands that run longer
(Claude Code: 2 min by default), and a killed wait would look like a failure. Waiting continues as
long as the agents it waits on (unplanned agents at the barrier, the holder of the claimed
function, agents holding or queued on a used function) are alive: the board records each agent's
last command, and a waiting command refreshes its own agent's record (at most once a minute). When
one of them has been silent for 30 min it may have crashed (it would never run `done`): the wait
stops with exit 1, `BLOCKED by <agent>`, and someone runs `release <agent>` if it crashed. A silent
agent is reported, never stripped of its claim: an agent can edit one function for a long time
without a command.

Agent names become folder names (`<project>-<agent>`): letters, digits, `-`, `_`, `.`, not
starting with `.`, and not `default` or `concord-gate`. `claim`, `done` and `finish` take no name:
they act for the agent whose workspace (or joined copy) they run in, and in the main folder they
exit 1 pointing to the workspace.

No setup is needed: an agent's first `plan`, run anywhere in the project, joins it, under the name
given (`plan <me> <modifies> <uses>`) or the first free `agent-<n>` (`plan <modifies> <uses>`).
With no board yet it sets the project up like `init` (base = the folder as it is); with a running
team it joins like `add`; plans run in the agent's own workspace just plan. A joining plan prints
`WORKSPACE <agent> <path>` (and `JOINED <agent>`), and the agent does the rest of its work there.
The main folder is never an agent's workspace, so a session editing there without concord keeps
its files. A plan from a workspace whose agent already landed or was released is refused.

A round ends when its review approves it (Landing, below): the landed agents leave the team
(workspace removed), the board's base moves to the landed change, and agents still working stay.
Once the team is empty the board starts over: the next agent's first `plan` snapshots the main
folder as it is (the landed code plus any edit made there since) as the new base, a change whose
parent is the last landed change. Edits made directly in the main folder therefore enter the next
round and the history; concord needs no separate commit step.

Agents that do not know about the board yet get told: `concord hook claude` is a Claude Code
`PreToolUse` hook on file edits (installed in the user settings). An edit in the main folder of a
project whose board has agents gets one note per session (sessions told are listed in
`hook-sessions` beside the board): who is working and how to join with `concord plan`. It never
blocks an edit and stays silent anywhere else.

| Command | Effect |
|---|---|
| `init [<agent>...]` | the project is the nearest folder with `.concord/` or `.git` (only its presence is read): a git repo nested in a project, such as a test fixture, is a project of its own; with neither, the current folder. Runs only in the project folder: from inside an agent's workspace it exits 1, since resetting the board would end every agent's round. Snapshots the main folder as the base change, uncommitted edits included (the main folder's last change when nothing changed since). Resets the board, removes workspaces of the previous and new team (files not in a change yet are kept as one first, `KEPT`), creates one workspace per agent at `<project>-<agent>` next to the project holding the base, prints `BASE <change>` and `WORKSPACE <agent> <path>`. With no agents it only makes the folder a project and records its base (no round runs; the next `plan` starts one). Idempotent. |
| `plan [<agent>] <modifies> <uses> -- <done when ...>` | joins first if needed (above); registers intent and the agent's done-criteria (required: what must be true of its own change; running its tests can be one) and its session (Landing); queue slot per modified function in plan order; idempotent (re-plan replaces the intent, keeps every earlier version of the criteria) |
| `claim <fn>` | waits until every agent has planned (barrier), then until this agent heads the queue for `fn`; prints `LATEST <change>` + note and merges that change into the agent's folder (`MERGED`), listing conflicted files (`CONFLICTS`) |
| `done <fn> <note>` | snapshots the workspace as one change described `<agent>: <fn>: <note>`, records the contract at that change, passes `fn` to the next in queue, lists callers that will be told; refused (exit 1, nothing recorded) while a listed conflict is unsettled |
| `finish` | snapshots leftover work as `<agent>: finish` (the agent's head is its last change), waits until nobody else holds/queues a function this agent uses; exit 0 = clean, exit 10 = unseen contract changes printed and already merged into the folder → settle conflicts, adapt, re-test, finish again; the round's last clean finish combines the round and asks this agent to review it (exit 12, Landing); exit 1 when the round cannot be combined or a listed conflict is unsettled |
| `review` | prints this agent's review assignment again (exit 12), or that it approved / has none (exit 0) |
| `land <evidence>` | the reviewer approves its part, with evidence per criterion (`file:line`), logged `APPROVE`; the last approval lands the round |
| `reject <agent> <reason>` | the reviewer rejects `agent`'s work: nothing lands, the reason goes to the author (Landing) |
| `status` | team, who planned, each function's holder and queue, who finished, minutes since each agent's last command |
| `show <change> [<file>]` | the change's paths, or one file's bytes (any unique id prefix) |
| `diff <from> <to>` | unified diff of every path that differs between two changes |
| `release <agent>` | orchestrator: removes a crashed agent from the team, its plan and every queue; each function it held passes to the next in queue |
| `add <agent>` | orchestrator: a new agent joins the running team (after `release`: replaces a crashed one); refused for a current member. Creates its workspace at the board's base, gives it a token (a running `serve` accepts it at once) and prints `ADDED`, `WORKSPACE`, and `JOIN <agent> <command>` while `serve` answers (`serve` records its URL in `serving.json` beside the board). Its silence clock starts now; until it plans, `claim`/`finish` wait for it as at the start. Its plan queues last; agents that already finished while using a function it modifies are named in its `PLANNED` output and the log: only the review checks them against its change |
| `log` | event log |

History only grows: a change is never rewritten, and other agents' work comes in by merge, never
by rebase. A change id is the hash of its content, so a change that left one workspace (another
machine fetched it, or another agent built on it) is the same change everywhere.

`claim`, `done` and `finish` act on the caller's workspace, so they must run inside the agent's
own workspace; anywhere else they exit 1 naming the right path.

## Store

concord keeps its files itself. What the protocol does with files is five operations, so that is
all the store does:

| Protocol step | Store operation |
|---|---|
| a workspace starts at the base (`init`, `plan`, `add`, join) | **materialize** a change into a folder |
| `done`, `finish`, before any merge | **snapshot** a folder into a change |
| `claim` (`LATEST`), `finish` (exit 10) | **merge** other changes into a folder (3-way, per file) |
| the gate combines a round; landing pulls it into the main folder | **merge** |
| remote agents | **sync** missing objects by hash |

There are no branches, refs, rebases or operation log. The board names every change that matters:
agent heads, contract changes, the combined result and the base. Rounds land one at a time under
`gate.lock`.

### Objects

Everything is content-addressed (BLAKE3, hex) under `<project>/.concord/objects/<2 hex>/<rest>`,
and immutable.

| Object | Content |
|---|---|
| blob | a file's bytes |
| tree | JSON: sorted `path → {blob, exec}` (symlinks stored as a blob of the target, `link: true`) |
| change | JSON: `{tree, parents: [change], agent, message}`. `parents` empty only for a project's first base |

A change id is the hash of its JSON; the board stores full ids, concord prints the first 12 hex
digits and accepts any unique prefix. A change is the unit the board refers to (`Contract.change`,
`heads`, `review.combined`, `landing.base`).

Durability: a write goes to `objects/tmp`, is synced (`fsync`), then renamed into place. A snapshot
syncs each directory it wrote to once and on macOS ends with one `F_FULLFSYNC`, which flushes the
drive cache, before the change id is handed to the board. The board file and `workspace.json` are
written the same way (sync the temp file, rename, sync the directory); each writer has its own temp
file (`<file>.<pid>.<n>.tmp`), so concurrent writers each land whole and the last rename wins. Objects are durable before
anything points at them.

### Workspaces

Every folder concord works in has `.concord/`. It holds `workspace.json`: `{name, head, merged,
store, conflicts}`. `head` is the folder's last change; `merged` lists changes merged in since then
(the next change gets `[head] + merged`, reduced to its tips, as parents); `store` is the path of the
project's `.concord` for agent workspaces (and the review checkout), absent in the main folder and
in joined copies (their own store); `conflicts` lists what the last merges left to settle.
`.concord/` also holds `index.json`, a stat cache (`path → size, mtime, ctime, inode, blob`; files
whose mtime or ctime is under 2 s old are not cached), so a snapshot hashes only changed files, and a `.gitignore` of `*`, so a git user never sees the folder. Which agent a
folder belongs to is its `name` when it has a `store` (or the agent of a joined copy); the main
folder belongs to no agent.

- **Snapshot** walks the folder with the `ignore` crate: `.gitignore`/`.ignore` files apply
  (whether or not git is present), `.concord`, `.git` and `.jj` are skipped at any depth,
  everything else is tracked. As in git, ignore rules apply to untracked paths only: a path of the
  head or merged trees that is still in the folder stays tracked, so ignoring it later never
  deletes it anywhere. Only regular files and symlinks are tracked (a FIFO, socket or device is
  skipped). A new file is in the next change, so no agent's file is ever left behind.
- **Materialize** writes a tree into a folder: deletes tracked paths the tree does not have (and
  folders left empty), then writes only paths whose blob differs. Untracked files the walk ignores
  are never touched.
- Removing a workspace (`init` again, landing) first keeps files not in a change yet as one
  (`KEPT <agent>'s uncommitted files as change <id>`).

### Merge

Functional core, `merge::plan(base, ours, theirs) -> Vec<(path, Step)>`, pure and tested without
files: per path, unchanged on one side → take the other (`Take`, or `Add` for a path ours never
had); same on both → keep; changed on both → `Text3(base, ours, theirs)` (an add on both sides is
`Text3` with an empty base) or a conflict (`ModifyDelete`, `Binary` when either side is a symlink).
The shell runs every deletion first, then the writes, so a folder replaced by a file (or the
reverse) merges:

- `Add` (and the modified side of a `ModifyDelete` that is theirs): when something on disk already
  holds the path (an untracked or ignored file, a folder ours added) or a file holds one of its
  folders, that stays and theirs is written under `<clash>.theirs`: a `Binary` conflict at the
  clashing path.
- `Text3`: `mergiraf merge <base> <ours> <theirs> -p <path> -o <file>`. Mergiraf merges any text,
  line-based where it has no grammar (exit 0 clean, exit 1 with conflict markers in the file).
  Bytes that are not UTF-8 get the `Binary` rule.
- `ModifyDelete`: the modified file stays; listed in `CONFLICTS` as a decision to make, never a
  block: keeping the file and deleting it both count as settled.
- `Binary`: ours stays, theirs is written next to it as `<path>.theirs`; listed.
- The merge base is the tree of the lowest common ancestor in the change graph (a common ancestor
  that is not an ancestor of another common one). After criss-cross merges there are several:
  as in git's recursive strategy they are merged pairwise (each pair against its own merge base,
  in a scratch folder `.concord/virtual-base`) into a virtual base; text conflicts in it keep their
  markers, other conflicts keep the modified or ours side.

Conflicted files are files the merge listed that still contain a `<<<<<<< ` line, or a `Binary`
whose `.theirs` file still exists (the agent deletes it to settle). `CONFLICTS`, the refusal of `done`/`finish`, and the gate's `unresolved
conflicts` read that list.

## Landing

Every concord user runs it from an AI agent (Claude Code, Codex, ...): concord never calls a model
of its own. Checks that need no judgment stay in concord; judgment goes back to the team's agents
as printed instructions plus an exit code, as exit 10 does. A passing test command is no landing
bar: the tests are written by the same agents, an agent that misreads its task writes a test that
agrees with it, and lost work takes its test with it.

**Combine.** The `finish` that leaves every agent on the team finished with nothing to re-check
combines the round, under `gate.lock` (one at a time: the board is read under the lock, so of two
finishes at once the second sees the review the first started and combines nothing). It fails if
any finished change carries no work of its own (its tree equals its only parent's, or it is the
base or an ancestor of it: its work was lost), takes the tips among the finished heads (a head that
is an ancestor of another is dropped), materializes the first in a folder of its own
(`<project>-concord-gate`), merges the others into it one at a time (Merge, above) and fails on
unresolved conflicts. The combined change (a single tip is its own combined change) and its
checkout stay until the round lands or is rejected. A failure is logged (`GATE FAIL <reason>`) and
kept on the board: every team member's `finish` prints `FAIL the round cannot be combined` with exit
1 until one of them finishes at a new head, which makes the combine due again. The main folder is
not touched, so others keep editing it.

**Review.** The last finisher's `finish` exits 12 with its assignment (`REVIEW REQUIRED`): the
combined change and its checkout, how to read them (`concord show`, `concord diff`), and two
checks, each against its own input.
- Every other agent's done-criteria against that agent's own head (`concord diff <base> <head>`):
  criteria describe the agent's own change; read against the combined code, a later agent's
  declared change makes them fail (measured). The first stated version is shown next to the
  current one when they differ (`CRITERIA CHANGED`), so a criterion bent to fit the code shows.
- The combined result against every contract change of the round: is every caller adapted.

The last finisher's own criteria go to another agent: the one whose own session concord can
continue on this machine, most recently active first. concord resumes that session with the
review prompt (`RESUMED`). With none (a one-agent team, remote agents, subagents sharing their
parent's session) the last finisher checks its own criteria too, logged `SELF-REVIEWED`, so the
weaker check stays visible. `init` teams follow the same rules; the orchestrator comes in only
where concord cannot route.

Each reviewer answers `concord land "<evidence>"` or `concord reject <agent> "<reason>"`. A head
that changes or leaves (`finish` again, `release`) cancels the waiting review (`REVIEW
CANCELLED`).

**Sessions.** `plan` records the agent's harness and session from the environment, no flag: Claude
Code's `CLAUDE_CODE_SESSION_ID`, Codex's `CODEX_THREAD_ID` (or `CODEX_SESSION_ID`). A session is
concord's to continue only when it was recorded on this machine and no other team member reported
the same one: subagents report their parent's session, which belongs to an orchestrator still
running. Continuing runs `claude --resume <id> -p <prompt>` / `codex exec resume <id> <prompt>` in
the agent's workspace, detached, output in `resume-<agent>.log` beside the board; when it cannot
start, concord prints the command instead.

**Approved** (every reviewer ran `land`): the main folder is pulled. `merge(base = the board's
base, ours = snapshot of the main folder, theirs = combined)` is written into the main folder:
only paths the round changed are touched, so uncommitted edits made there before or during the
round stay (a path both changed is merged by Mergiraf; leftover conflicts are printed as
`CONFLICTS` for whoever works there). The board's base becomes the combined change, the review
checkout and the landed agents' workspaces are removed, and the round ends for the landed agents
(`LANDED <change>`, `REMOVED workspace <agent>`, `ROUND <agents> landed; still working: ...`).
concord never reads or writes `.git`: a git user sees the landed round as uncommitted edits and
commits it with git.

Each landing appends one line to `.concord/ledger.jsonl`: `{at, base, landed, agents: [{agent,
head, criteria, contracts}], reviews: [{reviewer, evidence}]}`. The ledger is the project's
history; the change graph holds the files.

**Rejected**: nothing lands; the main folder stays exactly as it was, the agent workspaces stay,
the review checkout is removed. The reason goes to the author's session (resumed with the reason
and the worker protocol: fix, `concord finish`), so the agent that wrote the code fixes it with its
context; without a session of its own the reason is printed for whoever runs that agent. At most
two rejections per agent per round are sent back; the third stops with `BLOCKED <agent>: rejected
3 times` (exit 1) and the resume command, for a person to look.

`experiments/landing/run.sh` checks both paths from a git user's side: the landed files are
uncommitted edits, the user's edits from before and during the round survive, `.git` is byte for
byte unchanged; a rejected round leaves every file of the folder as it was.

## Agents on other machines: `concord serve`

`concord serve [port]` (default 7457) runs in the project after `init` and serves its board and
its objects over HTTP on every interface (meant for a tailnet). It prints one line per agent:
`JOIN <agent> concord join <url>`; the URL carries that agent's token (random, one per agent, made at
`init`, `add` or an agent's joining `plan`; kept in `.concord/tokens.json` beside the board, never
in it, so `api/board` and `status` do not expose it) and the host is the machine's Tailscale IPv4 when Tailscale runs, its hostname
otherwise.

- `<url>/api/whoami`, `api/board`, `api/command`: board commands, applied through the same store
  and `flock` as local agents, so local and remote agents mix freely. A blocking command holds its
  request until it can proceed. A command acts only for the token's agent (`command_agent`).
- `POST <url>/api/missing` (a list of object ids → the ones the server lacks), `POST
  <url>/api/objects` (a stream of objects; the server hashes each, refuses the whole upload on a
  mismatch, and makes them durable at once), `POST <url>/api/objects/get` (ids → those objects,
  one request). Uploads carry no ownership: ids are content hashes.
- `concord join <url>` on another machine makes `./<project>-<agent>` with a store of its own,
  fetches the base change (`whoami` names it) and materializes it, and records the server in
  `.concord/remote.json`. Worker commands there go to the server: `done` and `finish` upload the
  closure of the agent's change (the change, its tree, the blobs, and ancestors the server lacks;
  blobs first, then trees, then changes parents first) before the board records it, and the server
  refuses a `Done`/`Finish` whose change it cannot read completely; `claim`, `finish` and
  `review` fetch what they merge or name.
- Combining and landing run on the server machine. A remote reviewer reads the combined change
  through its copy (`concord review` fetches it, `concord show`/`diff` read it) and answers with
  `land`/`reject` over the API. Remote sessions are never resumed (they live on another machine).
  The copy on the other machine is left to its owner.

The board serializes functions, not files. Shared non-code files (docs, config) get one owner
per run, or the orchestrator edits them after the round lands.

Exit codes: `0` ok · `10` re-check required (`finish` printed unseen contract changes) · `11`
still waiting after 100 s: run the same command again · `12` review required (the output is the
assignment) · `1` protocol error, unsettled conflicts at `done`/`finish`, `BLOCKED` by a silent or
thrice-rejected agent, not in a workspace, or the round cannot be combined · `2` usage or setup
error (board or store I/O, Mergiraf failure, server unreachable or missing objects). Log timestamps
are UTC `HH:MM:SS`.

## Rust architecture

Functional core, imperative shell:

- `core` — pure state machine: `apply(Option<Board>, Command, now) -> Result<Step, Error>`, where
  `Step` is `Wait` (board unchanged, retry) or `Next(Board, Outcome)`. Barrier, queue order,
  idempotent plan, seen-set for contract notices. All protocol rules live here and are tested
  without filesystem or processes.
- `objects` — the object store (blobs, trees, changes by BLAKE3 id; batched durable writes) and
  the pure change-graph helpers (`ancestors`, `lowest_common_ancestors`, `heads`).
- `worktree` — a folder's `.concord` state: snapshot, materialize, commit, merge in, pull.
- `merge` — `plan` (pure, per path) and `apply` (Mergiraf, `.theirs` files, the conflict list).
- `diff` — the unified diff `concord diff` prints.
- `store` — the board under `flock`, durable atomic writes, the ledger, and `locate` (which folder
  a command runs in and where its project's `.concord` is).
- `serve` — the HTTP server (board API, object sync); `remote` — its client, used by worker
  commands in a joined copy.
- `team` — the project's main folder (found or started), the base snapshot, agent workspaces
  (create on `init`/`add`/joining `plan`, remove after landing).
- `gate` — combine (lost-work check, merge of the tips, review checkout), land (pull into the main
  folder), discard, the ledger entry.
- `cli` — CLI parsing, blocking/polling loops, exit codes. The crate is a library; the `concord`
  binary calls `concord::main`.

Exit codes are API: agents branch on them. Keep them stable.

End-to-end check: `experiments/cart-scenario/run.sh` replays the cart scenario with scripted agents
against the Rust binary; the reviewing agent's check runs the recorded tests on the combined change
and the landed result is diffed against `evidence/v2-final-*.py`. Scripted agents stand in for a
reviewer with a shell check (`experiments/review.sh`).

## Next decisions (open)

1. **Structured contracts**: proposal below (Structured contracts).
2. **Stable function identity**: claim keys follow renames (`total` → `subtotal` is the same entity).
3. **Speedup scenario**: 5+ agents on a real repo where most tasks are independent, to show v2
   does not over-serialize.
4. **Delivery beyond the skill**: the CLI ships with an agent skill (`skills/concord/`); an MCP
   server is the remaining option.
5. **Review with real agents**: LLM agents writing their own criteria at `plan` in the cart
   scenario (in `experiments/agent-review/` they were written for the experiment), then the
   ConflictBench merges Mergiraf got wrong (26) and right (27) in `bench/`, rule set before the
   run. A bug no criterion names is found only by chance, as with tests: the skill must make agents
   state criteria as contracts, not stories.

## Structured contracts (proposed, not built)

Problem: a contract is one free-text note, and nothing checks it. In the cart scenario B wrote
"returns int|float … a number" when the truth was "now always float". The note was vague, and it
was also wrong. C caught the float by reading code, not the note (findings, Known gaps after v2).
CooperBench saw a related failure with agents that coordinate by messages only. In its 50 failed
traces the main causes were wrong beliefs about the other agent's plan and broken commitments, and
turning chat off changed nothing. Its authors recommend checkable shared state such as pasted
signatures. They did not test that, or a shared board like this one (findings, External evidence).
This proposal is that recommendation, still untested.

**Command.** `done <fn>` takes these fields in place of the free-text note:

| Field | Content | Checked by |
|---|---|---|
| `--sig <file>:<from>-<to>` | where the definition is in the agent's working copy: the whole declaration, with decorators or attributes, the signature and any where clause. For a type, the whole definition. concord copies those lines into the contract, so the agent never pastes text. `--sig removed` means the function is gone | concord, before committing. The file must differ between the board's landing base and the workspace snapshot that `done` is about to record. The lines must contain the function's name as a whole word: `<fn>`, or `<new>` after a rename. Otherwise exit 1 and nothing is committed. `removed` is checked by the reviewer only |
| `--renamed-to <new>` | optional: the function's new name | concord: `<new>` appears in the `--sig` lines |
| `--was "<text>"` | what callers could rely on before | the reviewer |
| `--now "<text>"` | what callers can rely on now: a value seen by running the code (`repr(total(ITEMS, 20))` is `'20.0'`), not a belief about it | the reviewer |
| `--callers "<text>"` | what each caller must change, or `none` | the reviewer, against the combined code |

concord copies the lines itself. An agent therefore cannot record a partial or invented signature,
and multi-line signatures, braces and shell quoting stop being problems. The check only proves that
lines naming the function exist in a changed file. It does not prove they are `<fn>`'s definition
and not a call or a comment; the reviewer sees them.

`--now` asks for an observed value because B's note was wrong as well as vague: an agent believed
`int|float`. A required run of the code catches a wrong belief that a format alone cannot.
Behaviour such as int vs float in untyped code is invisible to a text check and to a parser alike,
so `--was`/`--now` stay declared text. The text check is chosen over a parser because it needs no
grammar per language.

`--renamed-to` gives decision 2 (claim keys follow renames) a declared field to key on, with no
name extraction from the code.

In a joined clone, `done` fetches from the server first (as `claim` and `finish` do), so the
landing base is known locally before the check.

**Output.** `finish` (exit 10), `claim` (`LATEST`) and the review print the fields, not a note:

```
NEW CONTRACT CHANGES in functions you call:
  total#2 (now subtotal) by agent-b at <change>
    sig:     cart.py:3  def subtotal(items, discount_pct=0):
    was:     returns int when every price is int
    now:     returns float always: subtotal([{"price": 10, "qty": 2}]) is 20.0
    callers: format the result; str() prints 20.0
```

The review's contract check stays "is every caller adapted", read against `callers`.

**Shell and core.** `Contract { id, agent, change, sig, renamed_to, was, now, callers }` replaces
`note`. `core::done` rejects empty fields (pure). The `--sig` check reads files, so it runs in the
shell before the commit, the same way `cmd_done` already checks a command the board would reject
before committing. The commit description becomes `<agent>: <fn>: <now>`.

**Changes with it:**
- `skills/concord/SKILL.md`: step 2 (`LATEST`, `done`), step 3 (exit 10), step 4 (the contract
  check), the contract-note guidance rewritten as one example per field plus what to do when
  `--sig` is rejected, and the quick-reference row.
- `cli.rs`: usage text, argument parsing, `LATEST` and review printing.
- The `done` row of the command table above.
- The scripted `done` calls in `experiments/{cart-scenario,landing,self-join,remote-agents}/run.sh`.

**How to know it worked (not runnable yet).** `bench/` applies gold patches and never reads a
contract, so it cannot measure this. The measurement needs a runner that does not exist yet: LLM
worker agents briefed with the concord skill, with code they write isolated from the host (today
`bench/` runs CooperBench code unsandboxed). Arms: free-text note vs structured contracts. Cases:
the cart scenario plus a CooperBench sample. The metric is unadapted callers, counted by a check
that never reads the contracts (hidden tests or a per-case caller check on the combined code, run
before review). Review verdicts are recorded separately. Set the model, the number of runs per
case and the thresholds before the first run, the way the Weave A/B did. Keep structured contracts
only if unadapted callers fall by that threshold and landed rounds do not fall.
