# Batch C — Consumption (Draft Discussion)

## Objective

Decide O3: both modes — lease and forward.

## Proposals

- Lease: `POST /v1/leases?host_key=` → `{name, url: socks5://name:1080, egress_ip?}` + explicit `DELETE /v1/leases/{id}` release + TTL reaper 30s fallback.
- Forward: `POST /v1/forward {method,url,headers,body}` → server picks healthy proxy via `next()` and proxies via `reqwest::Proxy::all`, returns `{status,headers,body}`. Optional `CONNECT` on `:8080` later.
- Advertisement: `socks5://name:1080` inside Docker network, `socks5://host:random_port` outside — config `advertise = "dns"|"host_port"`.

## Questions

- Require `host_key` sticky affinity now or simple round-robin MVP?
- Forward should stream (`Body` chunked) or buffer MVP?
