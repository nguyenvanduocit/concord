# concord

Coordination board and agent-reviewed landing for parallel coding agents working on one repository.

## What and why

Parallel coding agents rarely fail on merge conflicts. They fail on stale contracts: one agent
renames or changes a function while another still codes against the old version, and the merge
is textually clean. concord is a Rust CLI that each agent drives from its shell:

- **Board**: every agent states up front which functions it will modify and which it uses. Edits
  to one function are serialized (`claim` / `done`), each change carries a contract note, and an
  agent cannot finish until it has seen every contract change to a function it uses.
- **Landing**: when the round's last agent finishes, concord combines everyone's work and that
  agent reviews it against every agent's own done-criteria. Approved work is pulled into your
  project folder as uncommitted edits.
- **Store**: each agent works in its own copy of the project, backed by concord's own
  content-addressed store in `.concord/`. concord needs no VCS and never reads or writes `.git`;
  [Mergiraf](https://mergiraf.org) merges files.

concord never calls a model itself. Every judgment (adapting to a contract change, reviewing a
round) goes back to the agents as printed instructions plus an exit code.

## Evidence

On a three-agent scenario built to collide (a rename, a type change and a new caller of the same
function), every plain version-control setup (git, jj, GitButler, with or without Mergiraf)
landed broken code, at best 2/4 tests passing; claims and contract notes without a check of the
combined code reached 3/4; adding that check landed 4/4. A reviewer agent reading each agent's done-criteria, without running code,
rejected the main bugs 30/30 and landed correct code 20/20. Details, setups and caveats (one run
per setup; failure modes, not frequencies): [docs/findings.md](docs/findings.md).

## Requirements

- Rust >= 1.89 to build (uses `File::lock`).
- [Mergiraf](https://mergiraf.org) on `PATH` (tested with 0.20.0; `brew install mergiraf`).
- Agents that can run shell commands. The bundled skill and session resuming target Claude Code;
  Codex sessions are resumed too.
- Tested on macOS only. Remote agents expect a private network such as a Tailscale tailnet.

## Install

```bash
cargo install --path .                      # installs the concord binary
cp -R skills/concord ~/.claude/skills/      # the agent skill: orchestrator + worker protocol
```

Copy the skill again after editing it. Optional: a Claude Code `PreToolUse` hook that tells a
session editing the main folder, once, that agents are working and how to join. In
`~/.claude/settings.json`:

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Edit|Write|MultiEdit|NotebookEdit",
        "hooks": [{ "type": "command", "command": "\"$HOME/.cargo/bin/concord\" hook claude" }]
      }
    ]
  }
}
```

The hook never blocks an edit and stays silent outside a project with a working team.

## Quick start

**Orchestrator.** No setup is required. Give each agent a unique name, its task, and "follow the
concord skill (worker protocol)". Do not tell agents about each other's tasks; the board does
that. Give each shared non-code file (docs, config) to exactly one agent: the board serializes
functions, not files. Optionally create the workspaces up front:

```bash
concord init agent-a agent-b agent-c        # prints WORKSPACE <agent> <path> per agent
concord status                              # who holds / waits on what, who finished
concord log                                 # event log
```

**Worker.** Run from anywhere in the project:

```bash
concord plan agent-a total item_count -- "subtotal(ITEMS) == 25; nothing calls total any more"
# -> WORKSPACE agent-a <path>: cd there, work only there from now on
concord claim total                         # wait for your turn; merges the latest change to total
# edit, run the tests
concord done total "subtotal(items) -> int: sum of price*qty; CHANGED: renamed from total"
concord finish                              # handle the exit code, see below
```

`plan <modifies> <uses> -- <done-criteria>` takes comma lists (`-` for none): every function whose
code you touch, the functions you call and depend on, and what must be true of your change when
you finish. A reviewer checks your work against exactly those criteria. Claim one function at a
time. The full protocol is in [skills/concord/SKILL.md](skills/concord/SKILL.md).

## Exit codes

Agents branch on these; they are stable.

| Code | Meaning | What to do |
|---|---|---|
| 0 | ok | continue / report done |
| 10 | re-check required: functions you use changed (already merged into your folder) | settle `CONFLICTS`, adapt, re-test, `finish` again |
| 11 | still waiting after 100 s | run the same command again |
| 12 | review required: you finished last | review the round, then `concord land "<evidence>"` or `concord reject <agent> "<reason>"` |
| 1 | protocol error, unsettled conflict, `BLOCKED` (silent or thrice-rejected agent), round cannot be combined | read the message; `concord status`; `concord release <agent>` if it crashed |
| 2 | usage or setup error | read the message |

## Landing

The round lands by itself; there is nothing for the orchestrator to run.

1. The last `finish` combines every finished agent's work in a separate checkout and fails on
   lost work or unresolved conflicts.
2. That agent gets the review (exit 12): each agent's done-criteria against that agent's own diff,
   and the combined result against every contract change. Its own criteria go to another agent
   whose session concord can resume (with none, it checks them itself, logged `SELF-REVIEWED`).
3. `concord land "<evidence per criterion, file:line>"` approves; once every reviewer approved, the
   round is merged into your project folder as uncommitted edits (`LANDED <change>`). Your own
   edits made before or during the round stay, and the landed agents' workspaces are removed.
   Review and commit the result with your own VCS.
4. `concord reject <agent> "<reason>"` lands nothing and sends the reason back to that agent's
   session; the folder stays exactly as it was.

Every landed round is one line in `.concord/ledger.jsonl` (base, landed change, each agent's head,
criteria and contracts, each reviewer's evidence).

## Remote agents

```bash
concord serve                               # on the project machine; default port 7457
# prints: JOIN <agent> concord join <url>
concord join <url>                          # on the other machine: copies the project, prints WORKSPACE
```

From there the worker protocol is identical; concord moves files to and from the server by
content hash. Local and remote agents mix freely; combining and landing run on the serving
machine. `concord add <agent>` adds an agent to a running team.

## Docs

- [docs/design.md](docs/design.md): current behavior: protocol, store, merge, landing, remote
  agents, Rust architecture, open decisions.
- [docs/findings.md](docs/findings.md): what was tested and measured, with dates.
- [docs/research/](docs/research/): papers, tools and the 2026 landscape.
- [skills/concord/SKILL.md](skills/concord/SKILL.md): the agent skill.
- [experiments/](experiments/): end-to-end scenarios (`cart-scenario`, `landing`, `remote-agents`,
  `self-join`, each `run.sh`) and the agent-review experiment.
- [bench/](bench/): public merge benchmarks run through concord.
- [prototype/](prototype/): the Python/bash reference implementation behind the first 4/4 run.
- https://concord.aiocean.io: landing page and docs (source in [site/](site/)).

## Status

Version 0.1.0, a private tool under active development: commands, output and the on-disk format
change without migrations. The Rust CLI reproduces the cart scenario 4/4 end to end
(`experiments/cart-scenario/run.sh`). Open decisions (structured contracts, claim keys that follow
renames, a speedup scenario, review with real agents) are listed in
[docs/design.md](docs/design.md#next-decisions-open). Licensed under [MIT](LICENSE).
