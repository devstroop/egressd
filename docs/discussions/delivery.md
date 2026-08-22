# Infrastructure & Workflow — Compose, CI & Branching

## Context

`egressd` is greenfield Rust, built from scratch. Need reproducible delivery and strict branch pipeline that matches `main → develop → features → feat/* → feat/*-part`.

Covers **O5 Operability** + pipeline enforcement.

## Scope

- `compose.yaml` services & network
- CI (fmt/clippy/test without Docker)
- Branch/worktree hierarchy & hotfix flow

## Proposals

**Compose**
- Services: `egressd` (`build .`, `image egressd:local`, `9090:9090`, `volumes: /var/run/docker.sock`, `networks: egressd-net`) + `warpgate` image (`build warpgate/`, `warpgate:local`, not scaled directly — `egressd` scales via Docker API). Network `egressd-net` bridge (compose prefixes to `egressd_egressd-net`). Env `EGRESSD_IMAGE/PREFIX/COUNT/NETWORK/PORT` with `WARPGATE_*` compat for `proxyhub` drop-in. Keep `9090` (not `7000`) for compat; `7000` was proxbox draft leftover.

**CI**
- `cargo fmt -- --check` + `cargo clippy -- -D warnings` + `cargo test` on `push` to `main|develop|features` and PRs to `develop|features|feat/*`.
- Tests use fake Docker (pattern from `proxyhub/manager/tests/conftest.py`: `FakeContainersAPI`, `FakeVolumesAPI`, `FakeContainer`) — no daemon. `#[tokio::test]` with `bollard` mock trait.

**Branching & Worktrees (early creation done)**
```
main → develop → features | fixes | chores
                  └─ feat/<epic> (e.g., feat/api-v1)
                      └─ feat/<epic>-<part> (e.g., feat/api-v1-health)
```
- `features`/`fixes`/`chores` are plural categories; `feat`/`fix` singular leaves, `kebab-case`. `feat/<epic>-<part>` uses hyphen because git cannot have both `feat/x` and `feat/x/y`. Hotfix `hotfix/<slug>` from `main` → PR to `main` + back-merge `develop`.
- Worktrees `../egressd-wt-<dashed>` pre-created for levels 1-3 (`main`, `develop`, `features`, `feat/api-v1`, etc.) so discussion-phase work doesn't block `main`.

## Open Questions

- Keep `main` (current, modern) or rename to `master` to align `warpgate`/`ai-gateway`? (Current `main` is fine per prior Batch removal decision.)
- `warpgate` submodule pinned to tag or `main`? (Proposal: pin to `warpgate:local` build, submodule tracks `main` with `git submodule update --remote` on `chore`.)

## Out of Scope

- API contract (see Contract & Domain)
- Provider logic (see Orchestration)

## Decision Needed

Approve compose topology, CI, and confirm `9090` + `main` stay.
