# prototype

Reference behavior for the Rust implementation. Both ran the 4/4 PASS experiment.

- `coord.py` — the board (plan / claim / done / finish / log). Python stdlib, `flock` + JSON.
  Board dir is hardcoded to `/tmp/sw-v2-board`; run with `python3 -I coord.py ...`.
- `gate.sh` — combine agent heads with Mergiraf, run the combined tests (hardcoded pytest), PASS/FAIL.

The Rust port must keep the exit codes (0 / 10 / errors) and the protocol rules in `docs/design.md`.
Delete this folder once the Rust version reproduces the cart scenario result.
