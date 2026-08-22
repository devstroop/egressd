# ADR 001 — Branch & Worktree Naming

- Status: Accepted
- Date: 2026-08-22

## Context

Need strict branch pipeline with early higher-level worktrees.

## Decision

```
main (or master — repo picks one, alias not used)
 └─ develop (integration)
     └─ features | fixes | chores | other (category, early)
         └─ feat/<epic> | fix/<slug>       (epic)
             └─ feat/<epic>-<part>       (task, one issue)  # hyphen, not slash — git cannot have both feat/epic and feat/epic/part
```

- Full word category (`features`), short `feat`/`fix` leaf, `kebab-case` slug, no placeholders.
- Each leaf `feat/<epic>-<part>` = one GitHub Issue → PR to `feat/<epic>` → `features` → `develop` → `main` (hyphen avoids git file vs dir conflict where feat/epic exists).
- Worktree `../egressd-wt-<branch-dashed>` (e.g., `../egressd-wt-feat-api-v1-health`).
- Note: git cannot have branches `feat/x` and `feat/x/y` simultaneously (file vs dir), so task uses hyphen `feat/x-y`.

## Consequences

Early `features` worktree allows batched epic work before task leaves.
