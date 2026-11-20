---
name: concord
description: Use when several coding agents (subagents, parallel sessions) will edit the same repository at the same time, when splitting a task across parallel agents on one codebase, when you are a worker agent told to coordinate with concord, or when parallel agent work keeps landing with renamed/changed functions still called the old way. Covers the orchestrator (team setup, landing) and the worker protocol (plan with done-criteria, claim, done, finish, exit code 10, reviewing the round on exit code 12).
---

# concord: parallel agents on one repo

Parallel agents rarely break on merge conflicts; they break on stale contracts: one agent renames
or changes a function while another still codes against the old version. `concord` serializes
edits per function, tells callers about contract changes, and lands the combined result only when
the team's own agents reviewed it against each agent's done-criteria. It keeps its own file store
(`.concord/` in the project folder) and handles all of it: any folder works as is, with or without
git, and concord never reads or writes `.git`.

## Orchestrator

No setup is required: each agent's first `concord plan` joins the project on its own (worker
protocol, step 1), so you can hand agents tasks while others are already working. To give agents
their workspaces up front instead:

```bash
concord init agent-a agent-b agent-c      # from anywhere in the project
```

Prints one `WORKSPACE <agent> <path>` line per agent: a separate copy of the project for each.
Dispatch one agent per workspace. Each brief contains: a unique agent name, its workspace path (if
you ran `init`), its task, and "follow the concord skill (worker protocol)". Do not tell agents about each other's
tasks; the board does that. The board serializes functions only: give each shared non-code file
(docs, changelog, config) to exactly one agent, or edit it yourself after the round lands.

While they work: `concord status` (who holds/waits on what, who finished), `concord log` (events).

### Agents on other machines

```bash
concord serve                             # after init, from the project; keep it running
```

Prints `JOIN <agent> concord join <url>` per agent. Hand an agent on another machine (same
tailnet) its JOIN command instead of a WORKSPACE path: `concord join <url>` run there clones the
project into `./<project>-<agent>` and prints `WORKSPACE <agent> <path>`. From then on the worker
protocol is identical; concord moves files to and from the server. Local and remote agents mix
freely. Combining and landing run on the serving machine.

### Landing

Nothing to run: the round lands by itself. The round's last `finish` combines everyone's work and
asks that agent to review it (exit 12, worker protocol step 4); the other agents' sessions are
continued when the review or a fix needs them. Every reviewer's `concord land` lands the round like
a pull: the agents' changes are merged into your project folder as uncommitted edits (`LANDED
<change>`), your own edits stay, and the landed agents' workspaces are removed; agents still working
keep going and land in a later round. concord never touches git: review and commit the landed
edits with git yourself. Agents start from the folder as it is when the first agent joins (or at
`init`), uncommitted edits included. Every landed round is a line in `.concord/ledger.jsonl`.

Where concord cannot continue a session (agents on another machine, subagents that share your
session), it prints what the agent must do instead; relay it: a rejection reason goes to the agent
that wrote the code. `BLOCKED <agent>: rejected 3 times` is for you: read the reasons in
`concord log`. A rejected or failed round leaves your folder exactly as before.

## Worker protocol

After `plan`, run every command inside your workspace folder and edit files only there; concord
knows who you are from that folder.

1. **Plan before editing anything:** `concord plan <modifies> <uses> -- <done when ...>` — comma
   lists, `-` for none.
   Without a workspace yet, run it from anywhere in the project: it joins you, gives you a name
   (or use the one from your brief: `concord plan <name> <modifies> <uses>`) and prints
   `WORKSPACE <name> <path>`; `cd` there and work only there from now on.
   - `modifies`: every existing function/type/method whose code you will touch, even if its
     signature stays the same.
   - `uses`: existing functions you call whose behavior your code depends on.
   - after `--`: your done-criteria, what must be true of your own change when you finish. Another
     agent checks your work against exactly these, by reading it; nothing else is checked. State
     them as contracts with concrete values, not as a story: `subtotal(ITEMS, discount_pct=20) ==
     20; nothing calls total any more`, not "discount works". Running your tests can be one
     criterion if they are worth anything.
   Re-running `plan` replaces your plan; the reviewer still sees your first criteria next to
   changed ones.
2. **One function at a time.** Touch no file until `claim` has printed `CLAIMED`: an edit made
   before the claim lands on a base that may be about to change. For each function in `modifies`:
   - `concord claim <fn>` — waits for your turn. If another agent changed this function
     before you, it prints its note after `LATEST` and has already merged that change into your
     workspace (`MERGED`). `CONFLICTS <files>`: settle those files first: remove the conflict
     markers; for a `<file>.theirs` (the other side of a binary file or symlink, or a file the
     other side added where you already had an untracked one), put the right content in `<file>`
     and delete the `.theirs` file. A file one side deleted and the other changed is kept: decide
     whether it stays; keeping and deleting both settle it. `done` and `finish` refuse while
     conflict markers or a `.theirs` file remain.
   - Edit, run the tests.
   - `concord done <fn> "<contract note>"` — records your work so far as a change (you never
     commit yourself).
   - Finish this function with `done` before you claim the next one. Holding two claims can
     deadlock the team.
3. **Finish:** `concord finish`
   - exit 0, `FINISHED`: done. Report it.
   - exit 12, `REVIEW REQUIRED`: you finished last; review the round (step 4) before you report.
   - exit 10, `NEW CONTRACT CHANGES`: functions you use changed; those changes are already
     merged into your workspace (`MERGED`). Fix any `CONFLICTS` files, adapt your code to every note, re-run the
     tests, then `concord finish` again.
   - exit 11 from `claim` or `finish`, `WAITING for ...`: others are still working; run the
     same command again (each call waits up to 100 s, inside a tool's command timeout).
   - exit 1, `FAIL the round cannot be combined`: an unresolved conflict between agents' work or
     an empty finished change (work lost). Fix it in your workspace, then `concord finish` again.
   - exit 1: protocol error, or `BLOCKED by <agent>` (an agent you wait on has been silent for
     30 min): run `concord status`; if that agent crashed, `concord release <agent>`, then retry.
     Exit 2: setup error; read the message.

4. **Review** (exit 12; `concord review` prints the assignment again). Judge by reading what it
   prints: the combined result is checked out at the `COMBINED ... checked out at <path>` folder
   (on the serving machine), `concord show <change> [<file>]` prints a change's files or one file,
   `concord diff <base> <head>` an agent's own work:
   - `CHECK each agent's done-criteria`: each criterion against that agent's own work. A criterion
     is met only if the code, as written, satisfies it exactly.
   - `CHECK the combined result against every contract change`: is every caller of a changed function
     adapted to the new contract?
   - `CRITERIA CHANGED`: the agent rewrote its criteria after starting; check the first version
     was not bent to fit the code.
   Then exactly one of:
   - `concord land "<evidence per criterion, file:line>"`: approve. The round lands once every
     reviewer approved.
   - `concord reject <agent> "<what is wrong, file:line, what would satisfy the criterion>"`:
     nothing lands; concord sends the reason to that agent, who fixes it and finishes again.
   If you are resumed with a rejection: fix exactly that in your workspace, then `concord finish`.

### Contract note

The note is all a caller sees. Write the new contract, not the story:
`<name>(<params>) -> <return type>: <behavior>; CHANGED: <what callers must do differently>`

- Renamed: `subtotal(items) -> int: sum of price*qty; CHANGED: renamed from total`
- Type change: `subtotal(items, discount_pct=0) -> float; CHANGED: now always float, format before printing`

"returns a number" is not a contract; a caller cannot tell the result became a float.

## Quick reference

| Command | Who | Blocks | Prints | Exit |
|---|---|---|---|---|
| `init <agents...>` | orchestrator, optional | no | `INIT`, `WORKSPACE <agent> <path>` | 0 |
| `plan [<name>] <mods> <uses> -- <done when>` | worker | no | `PLANNED`; first time also `JOINED`, `WORKSPACE <name> <path>` | 0 |
| `serve [port]` | orchestrator | runs until killed | `JOIN <agent> <command>` | 0 |
| `join <url>` | worker on another machine | no | `WORKSPACE <agent> <path>` | 0 |
| `claim <fn>` | worker | up to 100 s per call, until all planned and your turn | `CLAIMED`, maybe `LATEST` + `MERGED` + `CONFLICTS` | 0 / 11 |
| `done <fn> <note>` | worker | no | `RELEASED`, next holder, callers told | 0 |
| `finish` | worker | up to 100 s per call, until nobody else holds/queues what you use | `FINISHED`, or `NEW CONTRACT CHANGES` + `MERGED`, or `REVIEW REQUIRED` + assignment, or `FAIL the round cannot be combined` | 0 / 10 / 11 / 12 / 1 |
| `review` | reviewer | no | the assignment again, or `NO REVIEW waiting` | 12 / 0 |
| `land <evidence>` | reviewer | no | `APPROVED`; last approval: `LANDED <change>`, `REMOVED workspace` | 0 |
| `show <change> [<file>]` / `diff <from> <to>` | anyone | no | a change's files or one file / unified diff | 0 |
| `reject <agent> <reason>` | reviewer | no | `REJECTED`, `RESUMED` or what to relay; third time `BLOCKED` | 0 / 1 |
| `status` / `log` | anyone | no | board / events | 0 |
| `release <agent>` | orchestrator | no | `RELEASED <agent>`, new holders | 0 |
| `add <agent>` | orchestrator | no | `ADDED`, `WORKSPACE <agent> <path>`, `JOIN` while serving | 0 |

## Common mistakes

| Mistake | Effect | Instead |
|---|---|---|
| Editing before `claim` prints `CLAIMED` | your edit races another agent's change to the same function | plan, claim, then edit |
| `modifies=-` while changing a function's body | nobody is queued behind you, so a parallel edit to that function goes unnoticed | list every function whose code you touch |
| Claiming a second function before `done` on the first | team deadlock | one claim at a time |
| Running a worker command outside your workspace | `ERROR run this inside your workspace` | `cd` to the path from `WORKSPACE` |
| Treating exit 11 as failure | your turn never comes | run the same command again |
| Agent name not in the team (never planned) | `ERROR ... not in the team` | use the exact name from the brief |
| Treating exit 10 as failure | caller ships code against a stale contract | fix conflicts, adapt, re-test, finish again |
| Vague note ("updated total") | callers miss the change | state the new contract and what CHANGED |
| Vague criteria ("feature works") | the reviewer has nothing to check against; a bug lands | concrete values and contracts the reviewer can verify by reading |
| Reporting done on exit 12 | the round never lands | review it: `land` or `reject` |
| Crashed agent while holding a claim | agents waiting on it stop after 30 min with `BLOCKED by <agent>` | orchestrator checks `concord status` (minutes since each agent's last command); if it crashed: `concord release <agent>`, then `concord add <new-agent>` and dispatch its task to the new agent |
