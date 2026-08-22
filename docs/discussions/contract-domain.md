# API Contract & Domain Boundaries

## Context

`egressd` is a control-plane daemon — data-plane traffic goes directly to proxy containers (SOCKS5 `:1080` / HTTP `:3128`). The API must be stable from day one; `proxyhub` is reference for semantics only, not source. WARP is one image (`warpgate:local` submodule), not the architecture. Stacking mental model (`ingress → L2 → 1..N L3`) is internal, not exposed.

Covers objectives **O2 Stable Management API** + **O6 Professional Separation**.

## Scope

- Versioned `/v1` (OpenAPI 3.1 as source of truth in `static/openapi.yaml`)
- Error envelope, validation, auth, task model
- Crate boundaries (`core` pure domain, `orchestrator` owns Docker, `api` owns HTTP, `forward` owns `reqwest` proxying)

## Proposals

**Versioning & Tasks**
- `/v1` only, no `/api/v1`. Mutations async: `202 Accepted` + `Location: /v1/tasks/{id}` + `GET /v1/tasks/{id}` polling (`Retry-After: 1`). `TASK_WORKERS=3`, `TASK_MAX_ITEMS=200`, `TASK_MAX_AGE=3600s`.
- Idempotency: `Idempotency-Key` header, dedup `86400s`, `409 IDEMPOTENCY_CONFLICT` if key reused with different `type`.

**Error Model**
- Envelope every response: `{"error":{"code","message","request_id"}}` + `X-Request-ID` header. `CORS *` for Swagger.
- Codes: `UNAUTHORIZED 401`, `VALIDATION_ERROR 422`, `POOL_AT_CAPACITY 409`, `PROXY_EXISTS 409`, `IDEMPOTENCY_CONFLICT 409`, `DOCKER_UNAVAILABLE 503`, `PROXY_NOT_FOUND 404`, `TASK_NOT_FOUND 404`.

**Validation (strict, 422 on mismatch)**
- `GET /pool?include=proxies|none`, `GET /proxies?healthy=true|false`, `POST /proxies/rotate {scope=unhealthy|all}`, `PATCH /pool {target:int}` with `name` regex `^[a-zA-Z0-9][a-zA-Z0-9_.-]*$` max 64.

**Crate Separation**
- `egressd-core`: `Endpoint`, `HealthResult`, `PoolSummary`, `TaskRegistry` — no `axum`/`bollard`/`reqwest`.
- `egressd-orchestrator`: Docker + health + reconciler.
- `egressd-api`: `axum` wiring + DTO mapping + OpenAPI.
- `egressd-forward`: `reqwest::Proxy::all` forwarding (depends on core + orchestrator).

## Open Questions

- `/v1` vs `/api/v1` prefix? (Proposal: `/v1` for drop-in with `proxyhub`.)
- `TaskRegistry` ownership: `core` (domain) vs `api` (transport)? (Proposal: `core` with `api` submitting workers.)

## Out of Scope

- `forward` streaming vs buffering (see Consumption discussion)
- Provider specifics (see Orchestration discussion)

## Decision Needed

Approve prefix, envelope, crate split before `feat/api-v1` work starts (`M1/M3`).
