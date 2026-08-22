# Pool Orchestration — Providers, Reconciliation & Health

## Context

Vendor-agnostic pool. WARP via `warpgate:local` is one `Provider` for MVP; future providers (`mullvad`, `wireguard`, embedded SOCKS) share same `target` reconciler. Control-plane only; `busy` guard avoids eviction races seen in `proxyhub`/`ai-gateway`. Per-proxy volumes preserve `reg.json` across restarts (Cloudflare re-registration is rate-limited).

Covers **O1 Agnostic Orchestration** + **O4 Desired-State Pool**.

## Scope

- `trait Provider` abstraction
- Desired-state `target_count` + `ensure_count`
- Health probes + health loop
- Persistence (volumes) & create retries

## Proposals

**Provider Abstraction (flat, no L2/L3 in public API)**
```rust
trait Provider: Send + Sync {
    fn kind(&self) -> &str; // "warp" for MVP
    async fn create(&self, ctx: &CreateCtx) -> Result<Endpoint>;
    async fn health(&self, ep: &Endpoint) -> HealthResult;
    async fn rotate(&self, ep: &Endpoint) -> Result<()>;
    fn mounts(&self, name: &str) -> Vec<Mount>; // e.g. {name}-data:/var/lib/cloudflare-warp
}
```
`CreateCtx { name, network, .. }`. `config.toml` `pool.provider = "warp"` (image `warpgate:local`), `prefix="egressd-"`, `count=3`, `max_pool=20`. Env `EGRESSD_*` with `WARPGATE_*` compat.

**Desired State**
- `target_count: AtomicUsize` + `pool: RwLock<Vec<Endpoint>>` + `ensure_lock: Mutex`. `POST /proxies` → `target++`, `DELETE` → `target--`, `PATCH /pool {target}` explicit. `ensure_count` converges under `ensure_lock` so health checker (30s loop) never races user scale.
- `CREATE_RETRIES=3` with new random suffix per attempt (`egressd-{hex}`), `PROXY_WAIT_TIMEOUT=60s` (`wait_for_healthy` polls `3s`), `TASK_WORKERS=3`.

**Health**
- `HealthResult { running, socks5, http, tunnel_connected, tunnel_detail }`, `healthy = running && socks5 && http && tunnel_connected`.
- `socks5_is_alive`: TCP + `05 01 00 → 05 00` (5s timeout); `http_is_alive`: `CONNECT cloudflare.com:443 → 200`; `tunnel_status`: `warp-cli status` via `bollard exec`.
- `_health_cycle` snapshots pool, probes by `container_id`, evicts only if `!running` + `!busy` + name gone (anchored `^name$` filter prevents substring match). `busy` set during `recreate_container` swap where `container_id` points at dying container.
- Volumes: `docker.types.Mount { target: /var/lib/cloudflare-warp, source: {name}-data }` + `{name}-cache` → `restart` keeps `reg.json`, `remove`/`scale-down`/`evict` calls `_cleanup_volumes`.

## Open Questions

- Port `ai-gateway` extras now or later: `unique_ips` (egress IP dedup), `IpBlacklist`, `max_concurrent` per proxy? (Proposal: defer post-MVP, keep MVP = `proxyhub` parity.)
- `tunnel_detail` string shape — keep proxyhub `detail` verbatim or structured enum?

## Out of Scope

- Lease vs forward (see Consumption)
- OpenAPI shape (see Contract & Domain)

## Decision Needed

Approve trait shape, triple probe, volume strategy before `feat/orchestrator-v1` (`M2`).
