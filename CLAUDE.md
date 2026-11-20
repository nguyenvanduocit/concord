# concord

Rust CLI: coordination board (`plan/claim/done/finish/log`) and agent-reviewed landing (`review/land/reject`) for parallel
coding agents, with its own content-addressed store (no jj, no git). Read `docs/design.md` before changing behavior; `docs/findings.md`
holds the evidence behind each design choice.

## Rules
- `docs/design.md` defines current behavior; a behavior change updates it in the same change.
- Private tool, no users: change formats, output and on-disk layout freely. No compatibility shims,
  no migrations. Licenses of borrowed code (jj, Mergiraf, Pijul, Weave, ...) are not a constraint.
- Exit codes are an API agents branch on: 0 ok, 10 re-check required, others are errors. Keep them stable.
- Functional core (`core`: pure board state machine, all protocol rules; `merge::plan` and the
  change-graph helpers in `objects`; all tested without I/O), imperative shell (`store` flock+JSON,
  `objects`/`worktree` the file store, `merge::apply` Mergiraf, `gate`, `serve`/`remote` HTTP, `cli`).
- Lints in `Cargo.toml` are deny; never silence them. No `unwrap`/`expect`/`panic`, no `unsafe`.
- Done = `cargo clippy --all-targets` clean + `cargo test` green + `experiments/cart-scenario/run.sh`,
  `experiments/landing/run.sh`, `experiments/remote-agents/run.sh` and
  `experiments/self-join/run.sh` passing.

## Environment
- Project lives on the external drive `/Volumes/Data`; cargo build-dir is already on that drive
  (`~/.cargo/config.toml`). The internal disk is nearly full: never write large artifacts to `/tmp` or `~`.
- Tools used: Mergiraf 0.20.0 (Homebrew) merges every file concord merges; git only as the user's
  own tool in experiment fixtures. Building needs Rust >= 1.89 (`File::lock`; rustup's toolchain
  on PATH). GitButler was evaluated and rejected for shared-directory parallel work (findings).
