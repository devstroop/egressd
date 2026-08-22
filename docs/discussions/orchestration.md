# Orchestration — Provider Trait, Pool Reconciler, Health & Volumes

## Objective

Decide O1 + O4: provider trait, pool reconciler, health, volumes.

## Proposals

- `trait Provider { kind()->&str; create(ctx)->Endpoint; health(ep)->HealthResult; rotate(ep)->(); mounts(name)->Vec<Mount> }` — WARP for MVP, pluggable.
- Desired-state `target_count` (proxyhub `target`) — `POST /proxies` bumps, `DELETE` lowers, `PATCH /pool {target}`, `ensure_count` converges with `ensure_lock` Mutex.
- Triple health `socks5_is_alive` handshake + `http_is_alive` CONNECT + `warp_status` exec, `busy` guard prevents eviction mid-recreate.
- Per-proxy volumes `{name}-data`/`{name}-cache` for `reg.json` preservation, `CREATE_RETRIES 3`, `PROXY_WAIT_TIMEOUT 60s`, `TASK_WORKERS 3`.

## Questions

- Stacking mental model `L2→L3` — keep flat trait now, layer internally later? (Answer: yes, not highlighted).
- `ai-gateway` features to port now vs later: `unique_ips`, `blacklist`, `max_concurrent`?
