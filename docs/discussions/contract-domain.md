# Contract & Domain — Stable /v1 API & Crate Separation

## Objective

Decide O2 + O6: stable `/v1` API versioning, OpenAPI 3.1 source of truth, crate boundaries, error envelope.

## Proposals

- Versioned `/v1` only, `202 + Location: /v1/tasks/{id}` for mutations (proxyhub semantics).
- Error codes: `UNAUTHORIZED 401`, `VALIDATION_ERROR 422`, `POOL_AT_CAPACITY 409`, `PROXY_EXISTS 409`, `IDEMPOTENCY_CONFLICT 409`, `DOCKER_UNAVAILABLE 503`, `PROXY_NOT_FOUND 404`, `TASK_NOT_FOUND 404`.
- Crate split: `core` (no axum/bollard), `orchestrator` (bollard), `api` (axum), `forward` (reqwest).
- Strict validation: `include=proxies|none`, `healthy=true|false`, `scope=unhealthy|all`, `name` regex `^[a-zA-Z0-9][a-zA-Z0-9_.-]*$` max 64.
- `X-Request-ID` on every response, `Idempotency-Key` dedup `86400s`.

## Questions

- `/v1` vs `/api/v1` prefix?
- `core` should own `TaskRegistry` or `api`?
