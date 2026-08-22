# ADR 002 — Project Name `egressd`

- Status: Proposed
- Date: 2026-08-22

## Context

Need vendor-agnostic name, not WARP-biased. `proxyhub` is experimental tainted, `proxbox` informal.

## Decision

`egressd` — egress daemon, Unix `d` style, covers stacked mental model `ingress TP → L2 → 1..N L3` internally without highlighting architecturally.

## Alternatives

- `proxbox` — continuity, but toy
- `proxyplane` — control-plane, broader, professional runner-up
- `warpbox` — rejected WARP-biased

## Consequences

Binary `egressd`, crate `egressd-*`, service `egressd:9090`, env `EGRESSD_*` with `WARPGATE_*` compat.
