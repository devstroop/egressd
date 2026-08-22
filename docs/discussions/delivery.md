# Delivery — Compose, CI & Branching Strategy

## Objective

Compose, CI, branch/worktree workflow.

## Proposals

- `compose.yaml` services `egressd` (build `.`, `9090:9090`, `docker.sock`, `egressd-net`) + `warpgate` image `warpgate:local` (build `warpgate/` submodule).
- CI `cargo fmt --check` + `clippy -D warnings` + `cargo test` with fake Docker (no daemon).
- Branch hierarchy `main → develop → features → feat/<epic> → feat/<epic>/<part>` early worktrees for levels 1-3.

## Questions

- Port `9090` (proxyhub compat) or `7000` (proxbox draft)?
