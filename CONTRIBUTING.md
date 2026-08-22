# Contributing — egressd

## Pipeline

1. **Repo** — this scaffold (`main`, `develop`, `features` worktrees ready).
2. **Objectives** — `README.md` O1-O6.
3. **Discussions** — GitHub Discussions (see `docs/discussions/` drafts).
4. **Plan** — ADR in `docs/adr/`.
5. **Issues** — GitHub Issues, validated, hardened, labelled, milestoned.
6. **Worktree/Branch** — per hierarchy below, no direct push to `main`/`develop`.

No code until milestones/issue hardened.

## Branch Hierarchy

```
main (or master — pick one, protected)
 └─ develop (integration, protected)
     └─ features | fixes | chores | other  (category, early, e.g., features)
         └─ feat/<epic> | fix/<slug>       (epic, e.g., feat/api-v1)
             └─ feat/<epic>-<part>       (task, e.g., feat/api-v1-health-probe)  # hyphen not slash
```

- Full word for category (`features` not `feat`), short `feat`/`fix` for leaf. `kebab-case` slug only. No `asdfghjkl` placeholders.
- Each leaf `feat/<epic>-<part>` = one Issue → PR to `feat/<epic>` → `features` → `develop` → `main` (hyphen avoids git file/dir conflict).
- Hotfix: `hotfix/<slug>` from `main` → PR to `main` + back-merge `develop`.

## Worktrees

```bash
git worktree add ../egressd-wt-main main
git worktree add ../egressd-wt-develop develop
git worktree add ../egressd-wt-features features
git worktree add ../egressd-wt-feat-api-v1 feat/api-v1
git worktree add ../egressd-wt-feat-api-v1-health feat/api-v1-health
```

## Milestones

- **M0 Scaffolding** — workspace, CI, `features` worktree, config, `warpgate` submodule
- **M1 Core Domain** — Endpoint/Health/PoolState/TaskRegistry/schemas
- **M2 Orchestrator** — Docker, lifecycle, health loop, volumes
- **M3 API v1** — axum routes, 202+Location, idempotency, auth, OpenAPI, docs
- **M4 Forward** — lease + forward mode
- **M5 Hardening** — tests without Docker, clippy, tracing

Issues carry `type:feat|fix|chore`, `area:core|orchestrator|api|forward`, `milestone` label.

## Code Style

- `cargo fmt` + `cargo clippy -- -D warnings` required.
- `core` crate has no `axum`/`bollard`/`reqwest` deps.
- Tests use fake Docker (no daemon), `#[tokio::test]`.

## Commit

Conventional: `feat(api): add health endpoint`, `fix(orchestrator): busy guard race`, `chore: rename master to main`.
