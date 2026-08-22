# Proxy Access — Lease & Forward Modes

## Context

Consumers (`ai-gateway` today, others later) need two ways to use the pool, both control-plane: (A) get a `socks5://` URL and dial themselves, (B) ask `egressd` to forward. Both modes stay out of `ai-gateway` until `egressd` is stable. No fallback — pool degraded → `503`.

Covers **O3 Both Consumption Modes**.

## Scope

- Lease API (new, `proxyhub` has no lease)
- Forward API (convenience, not hot-path)
- Endpoint advertisement (DNS vs host-port)

## Proposals

**Lease (A)**
- `POST /v1/leases { host_key?: string }` → `{ id, name, endpoints: { socks5: "socks5://name:1080", http: "http://name:3128" }, status, created_at, uptime_s }` via `next()` (round-robin healthy, `busy` skipped). Explicit `DELETE /v1/leases/{id}` release + TTL reaper (`30s`) fallback so crashed clients don't leak `in_flight`-style counts (if `max_concurrent` added later).
- No `GET /v1/proxies/next` — `leases` resource makes lifecycle explicit.

**Forward (B)**
- `POST /v1/forward { method, url, headers?, body?, timeout_ms? }` → `egressd` picks healthy proxy via same `next()` and does `reqwest::Client::builder().proxy(Proxy::all(socks5_url)).timeout(120s).send().await` → `{ status, headers, body }`.
- Streaming follow-up `POST /v1/forward/stream` returning chunked `Body` (not buffered) as `feat/forward-mode-stream`. MVP buffers.
- Later `CONNECT` on `:8080` (`curl -x http://egressd:9090`) — deferred, `forward` covers non-Rust clients now.

**Advertisement**
- Inside Docker `egressd-net`: `socks5://name:1080` (Docker DNS). Outside: `socks5://host:random_port` (proxyhub `create` used `HostConfig` `PortBinding` random; `egressd` will keep random host port but prefer DNS when caller is on same network). Config `advertise = "dns"` (default) | `"host_port"`.

## Open Questions

- Require `host_key` sticky affinity now or simple round-robin MVP? (Proposal: round-robin MVP, sticky as `feat/forward-mode-sticky` follow-up.)
- Forward timeout `120s` vs per-request `timeout_ms`? (Proposal: per-request overrides, default `120s` like `proxyhub`.)
- Lease needs `egress_ip` field now? (Proposal: no, add when `unique_ips` lands.)

## Out of Scope

- Provider selection (see Orchestration)
- Auth on lease/forward (covered by `MANAGER_API_KEY` Bearer, exempt health/openapi/docs)

## Decision Needed

Approve `leases` resource + `forward` JSON shape before `feat/forward-mode` (`M4`).
