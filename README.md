# egressd

Vendor-agnostic proxy orchestration control-plane — WARP is one provider, not the architecture.

> **Status:** Scaffold / Discussion phase — no stable API yet. See [Discussions](../../discussions) for objectives.

`egressd` owns the proxy lifecycle (Docker/bollard, health, scaling, rotation) and exposes a stable Management API. Data-plane traffic goes directly to proxy containers (SOCKS5 / HTTP) — the daemon is control-plane only except for the optional `POST /v1/forward` convenience.

`ai-gateway` and other consumers stay agnostic — they consume `egressd` via lease (`socks5://name:1080`) or forward (`http://egressd:9090/v1/forward`).

## Objectives

- **O1 — Agnostic Orchestration:** `trait Provider` — WARP (`warpgate:local` + 3proxy) for MVP, pluggable others later, same pool semantics.
- **O2 — Stable Management API:** Versioned `/v1`, OpenAPI 3.1 source of truth, `202 Accepted + Location: /v1/tasks/{id}` for mutations, strict `422` validation, `409` idempotency, `X-Request-ID` envelope.
- **O3 — Both Consumption Modes:** Lease (`POST /v1/leases` → `socks5://` + `DELETE /v1/leases/{id}` release) and Forward (`POST /v1/forward`, optional `CONNECT`).
- **O4 — Desired-State Pool:** `target` reconciler, per-proxy volumes preserving registration, triple health (SOCKS5 handshake + HTTP CONNECT + tunnel status), `busy` guard.
- **O5 — Private-Net Basic:** `MANAGER_API_KEY` Bearer, `AUTH_EXEMPT` health/openapi/docs, permissive CORS — hardening later.
- **O6 — Professional Separation:** `core` (no axum/bollard), `orchestrator` (bollard), `api` (axum), `forward` (reqwest) — tested with fake Docker, `clippy -D warnings`.

> Mental model only: `ingress TP proxy → L2 → 1..N L3` stacking is internal, not architecturally highlighted.

## Repo Layout

```
egressd/
  Cargo.toml (workspace)
  crates/
    egressd-core/          # domain: Endpoint, Health, PoolState, TaskRegistry
    egressd-orchestrator/  # Docker client, lifecycle, reconciler, health loop
    egressd-api/           # axum control-plane, DTOs, OpenAPI
    egressd-forward/       # forward proxy (optional)
  warpgate/ (submodule https://github.com/devstroop/warpgate — image only)
  compose.yaml
  config.example.toml
  docs/adr/
  .github/
```

## Development Pipeline

Strict flow, no code until milestones hardened:

```
repo → objectives (this README) → Discussions → Plan → Issues (GitHub) → Milestones → worktree/branch
```

Branch hierarchy (early creation):

```
main (or master — pick one, protected)
 └─ develop (integration)
     └─ features (category, early)
         ├─ feat/<epic>              # e.g., feat/api-v1, feat/orchestrator-v1
         └─ feat/<epic>-<part>       # e.g., feat/api-v1-health-probe (hyphen, slash would conflict feat/epic)
     └─ fixes / chores / other (category peers of features)
         └─ fix/<slug> / chore/<slug>
```

Each leaf `feat/<epic>/<part>` is one GitHub issue → PR to `feat/<epic>` → PR to `features` → PR to `develop` → `main` on tag.

Worktrees: `git worktree add ../egressd-wt-<branch-dashed> <branch>`

## Quick Start (after M3)

```bash
cp config.example.toml config.toml
docker compose up -d --build
curl http://localhost:9090/v1/health | jq
curl -X POST http://localhost:9090/v1/proxies -H "Idempotency-Key: k1" | jq
curl http://localhost:9090/v1/tasks/<id> | jq
```

## Reference Only

- `proxyhub/manager` — experimental Python control-plane (reference for API semantics, not source)
- `ai-gateway/src/warpgate` + `src/proxy` — advanced pool features (blacklist, autoscale) to be ported selectively post-MVP

## License

MIT
