# agent-review: can an agent replace the test gate?

Falsification run for the proposal in `docs/design.md` ("Proposal: agent-judged landing"): instead
of running the project's tests, the gate hands a review to an agent. This experiment gives a
tool-less agent exactly what that review would print — every agent's done-criteria, the round's
contract notes, the combined code — and asks for `LAND` or `REJECT <agent>: <reason>`. No code is
run by the reviewer.

## Cases (the cart scenario, `../cart-scenario/`)

| Case | Combined `receipt()` | Contract notes | Truth (pytest, checked by `run.py`) |
|---|---|---|---|
| `bug-float-vague` | `total {subtotal(items)}` → prints `25.0` | the notes recorded in the LLM run (B: "returns a number") | reject |
| `bug-float-precise` | same | B: "now always float, format before printing" | reject |
| `bug-rename` | `total {total(items)}` → `NameError` | recorded notes | reject |
| `ok-v2` | `evidence/v2-final-cart.py` | recorded notes | land |
| `ok-alt` | `total {subtotal(items):g}` | recorded notes | land |

Done-criteria are written from each agent's task text (`../cart-scenario/README.md`), as an agent
would state them at `plan`, before seeing other agents' work.

## Rule, fixed before the first run

Per model, 5 runs per case. The design survives for that model iff every bug case is rejected in at
least 4/5 runs and every ok case lands in at least 4/5 runs. `bug-float-vague` landing in 2 or more
runs falsifies it outright: that is the bug the test gate exists to catch.

## Run

```
python3.12 experiments/agent-review/run.py [runs-per-case] [models...]   # default: 5 haiku sonnet
```

Needs `claude` logged in. Runs from `/Volumes/Data/concord-review-runs` with no user settings,
hooks, MCP servers or tools. Results: `results.md` here.

## Run 1 (2026-11-19): rule says no, both models

`results-run1.md`. Every bug case was rejected in 30/30 reviews, including `bug-float-vague`, the
case the rule could falsify outright. The failures are false rejections, and both trace to the
experiment, not the reviewer:

- `ok-alt` is a real bug the tests miss: `:g` prints a total of 1234567 as `1.23457e+06` and
  12345.67 as `12345.7`. Sonnet rejected it 5/5 with exactly that reason (haiku 3/5). The label
  "land" came from the four tests, the weak oracle this experiment questions.
- haiku rejected `ok-v2` in 2/5 runs on agent-b's criterion "calls without a discount behave as
  before". B's task requires `gross * (1 - discount_pct / 100)`, always a float: the criterion
  written for this experiment contradicted B's own task, and the reviewer held B to the text.

## Run 2: corrected cases, rule fixed before the run

- agent-b's criterion is its task text, nothing added.
- Truth = the four tests plus hidden checks the reviewer never sees: a one-item cart totalling
  1234567 prints `total 1234567`; one totalling 12.5 prints `total 12.5`.
- `ok-alt` becomes a correct alternative (`int(t) if t == int(t) else t`, inline); the `:g` version
  stays as `bug-latent-g`, truth reject.

Same rule as run 1, same models, 5 runs per case. Results: `results.md`.

## Run 2 (2026-11-19): rule says no, both models

`results-run2.md`. The three main bug cases: 30/30 rejected. Sonnet landed both ok cases 10/10.
Two failure modes remain:

- haiku rejected `ok-v2` 3/5 on agent-a's criterion "renamed with unchanged behavior": checked
  against the combined code, where agent-b later changed `subtotal` and declared it
  (`total#2`). A design flaw, not a case error: criteria describe an agent's own change, the review
  read them against everyone's.
- `bug-latent-g` (no criterion or contract mentions large totals) is caught by chance: sonnet 5/5
  in run 1, 2/5 here; haiku 4/5. The four tests catch it 0/5.

## Run 3: criteria checked against each agent's own head

The fix the board makes mechanical: each agent's done-criteria are checked against that agent's
own head (concord has every head); the combined result is checked against the round's contract
changes. Heads: agent-a = base with the rename; agent-b = agent-a's head plus the discount;
agent-c = the combined result.

Rule, fixed before the run. The question is whether the reviewer can replace the test gate, so
the test gate is the bar: per model, every case the four tests reject is rejected in at least 4/5
runs, every case they land is landed in at least 4/5 runs, `bug-float-vague` landing 2 or more
times falsifies outright. `bug-latent-g` (the tests land it: 0/5) is reported, not gated.

## Run 3 (2026-11-19): survives, both models

`results.md`. Every case the test gate decides was decided the same way in 50/50 reviews: the
three bug cases rejected 15/15 per model, `ok-v2` and `ok-alt` landed 10/10 per model.
`bug-latent-g`: sonnet 0/5, haiku 2/5 rejected. Scoping each agent's criteria to its own head
removed the false rejections and also most of the off-criterion scrutiny that caught the latent
bug in runs 1 and 2: with criteria scoped, the reviewer checks what was promised, as a test does.

Limits: one scenario, 5 runs per case, done-criteria written for the experiment rather than by
agents at `plan`, and a reviewer with no tools (a real agent can run the code).
