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
             └─ feat/<epic>/<part>         (task, one issue)
```

- Full word category (`features`), short `feat`/`fix` leaf, `kebab-case` slug, no placeholders.
- Each leaf `feat/<epic>/<part>` = one GitHub Issue → PR to `feat/<epic>` → `features` → `develop` → `main`.
- Worktree `../egressd-wt-<branch-dashed>`.

## Consequences

Early `features` worktree allows batched epic work before task leaves.
