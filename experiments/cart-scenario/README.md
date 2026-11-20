# cart scenario

Three agents, one file, built to collide. Results per setup: `../../docs/findings.md`.

`base/` is the starting repo content. Each agent gets one task and is not told about the others:

| Agent | Task | Test it adds |
|---|---|---|
| A | rename `total` → `subtotal`, body unchanged | `assert cart.subtotal(ITEMS) == 25` |
| B | `total(items, discount_pct=0)` returning `gross * (1 - discount_pct / 100)` (follow a rename if told) | discount 20 on ITEMS == 20 |
| C | `receipt(items)` → `"<count> items, total <total>"` reusing existing functions | `assert cart.receipt(ITEMS) == "3 items, total 25"` (fixed spec) |

Correct combined result: 4 tests pass, `receipt` calls `subtotal`, and prints `25` not `25.0`.

`evidence/` holds the jj + coord v2 run (the only 4/4 PASS):
- `v2-board-log.txt`, `v2-board-state.json` — the full coordination log, including the one `REPAIR`
- `v2-history.txt` — resulting jj history base → A → B → C → gate
- `v2-final-cart.py`, `v2-final-test_cart.py` — the combined code the gate tested

Agents were Claude Haiku subagents briefed with the protocol text from `../../docs/design.md`.

`run.sh` replays the scenario against the Rust binary with scripted agents (same edits as the
recorded run, protocol driven by `concord`); the last finisher reviews the combined result by
running the recorded tests on it and approves, and the landed result is diffed against
`evidence/v2-final-*.py`. Output goes to `target/cart-run/`. Needs mergiraf and a Python with
pytest (`PYTHON=python3.12` by default).
