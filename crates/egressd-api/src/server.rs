use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use egressd_core::endpoint::Endpoint;
use egressd_core::task::{TaskError, TaskRegistry, TaskStatus};
use egressd_orchestrator::docker::fake::FakeDocker;
use egressd_orchestrator::pool::Pool;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::AtomicU32;
use std::sync::{Arc, Mutex};

const MAX_POOL: usize = 20;

#[derive(Clone)]
pub struct AppState {
    pub pool: Arc<Pool>,
    pub docker: Arc<FakeDocker>,
    pub registry: Arc<TaskRegistry>,
    pub api_key: Option<String>,
    pub rate_limit: Arc<Mutex<HashMap<String, Vec<std::time::Instant>>>>,
    pub consecutive_fallbacks: Arc<AtomicU32>,
}

impl AppState {
    pub fn new(target: usize, api_key: Option<String>) -> Self {
        Self {
            pool: Arc::new(Pool::new(target)),
            docker: Arc::new(FakeDocker::new()),
            registry: TaskRegistry::new(),
            api_key,
            rate_limit: Arc::new(Mutex::new(HashMap::new())),
            consecutive_fallbacks: Arc::new(AtomicU32::new(0)),
        }
    }
}

// ── Error envelope ──────────────────────────────────────────────────────

pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
    request_id: String,
}

impl ApiError {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>, rid: &str) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            request_id: rid.to_string(),
        }
    }

    fn unauthorized(rid: &str) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "UNAUTHORIZED", "unauthorized", rid)
    }

    fn validation(msg: impl Into<String>, rid: &str) -> Self {
        Self::new(StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_ERROR", msg, rid)
    }

    fn not_found(code: &'static str, msg: impl Into<String>, rid: &str) -> Self {
        Self::new(StatusCode::NOT_FOUND, code, msg, rid)
    }

    fn conflict(code: &'static str, msg: impl Into<String>, rid: &str) -> Self {
        Self::new(StatusCode::CONFLICT, code, msg, rid)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = json!({
            "error": {
                "code": self.code,
                "message": self.message,
                "request_id": self.request_id,
            }
        });
        let mut resp = (self.status, Json(body)).into_response();
        if let Ok(v) = HeaderValue::from_str(&self.request_id) {
            resp.headers_mut().insert("X-Request-ID", v);
        }
        resp
    }
}

type ApiResult = Result<Response, ApiError>;

// ── Middleware: auth + rate limit + request id ───────────────────────────

fn check_auth(state: &AppState, headers: &HeaderMap, client_ip: &str) -> Result<(), ApiError> {
    let Some(key) = state.api_key.as_ref() else {
        return Ok(());
    };
    let auth = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let expected = format!("Bearer {key}");
    if constant_eq(auth.as_bytes(), expected.as_bytes()) {
        return Ok(());
    }
    // Rate-limit failed attempts per IP: MAX 10 / 60s window
    const RATE_LIMIT: usize = 10;
    const WINDOW_SECS: u64 = 60;
    {
        let mut store = state.rate_limit.lock().unwrap();
        let now = std::time::Instant::now();
        let records = store.entry(client_ip.to_string()).or_default();
        records.retain(|t| now.duration_since(*t).as_secs() < WINDOW_SECS);
        if records.len() >= RATE_LIMIT {
            return Err(ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "RATE_LIMITED",
                "too many requests",
                "",
            ));
        }
        records.push(now);
    }
    Err(ApiError::unauthorized(""))
}

fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn new_request_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..16].to_string()
}

fn with_rid(headers_out: &mut HeaderMap, rid: &str) {
    if let Ok(v) = HeaderValue::from_str(rid) {
        headers_out.insert("X-Request-ID", v);
    }
}

fn accepted(task_id: &str, body: Value) -> Response {
    let mut resp = (StatusCode::ACCEPTED, Json(body)).into_response();
    resp.headers_mut()
        .insert("Location", HeaderValue::from_str(&format!("/v1/tasks/{task_id}")).unwrap());
    resp
}

fn ok_json(v: Value, rid: &str) -> Response {
    let mut resp = (StatusCode::OK, Json(v)).into_response();
    with_rid(resp.headers_mut(), rid);
    resp
}

// ── Handlers ─────────────────────────────────────────────────────────────

async fn health(State(state): State<AppState>) -> Response {
    let rid = new_request_id();
    let s = state.pool.summary(false).await;
    let status = if s.pool_size == 0 {
        "initializing"
    } else if s.degraded == 0 {
        "ok"
    } else {
        "degraded"
    };
    let mut resp = (
        StatusCode::OK,
        Json(json!({
            "status": status,
            "pool_size": s.pool_size,
            "healthy": s.healthy,
            "degraded": s.degraded,
        })),
    )
        .into_response();
    with_rid(resp.headers_mut(), &rid);
    resp
}

#[derive(serde::Deserialize)]
struct PoolQuery {
    include: Option<String>,
}

async fn get_pool(
    State(state): State<AppState>,
    Query(q): Query<PoolQuery>,
    headers: HeaderMap,
) -> ApiResult {
    let rid = new_request_id();
    check_auth(&state, &headers, "client")?;
    match q.include.as_deref() {
        None | Some("proxies") => {}
        Some("none") => {}
        Some(_) => {
            return Err(ApiError::validation(
                "include must be 'proxies' or 'none'",
                &rid,
            ))
        }
    }
    let include = q.include.as_deref() != Some("none");
    let summary = state.pool.summary(include).await;
    Ok(ok_json(crate::schemas::to_pool_dict(&summary, include), &rid))
}

async fn patch_pool(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> ApiResult {
    let rid = new_request_id();
    check_auth(&state, &headers, "client")?;

    // Idempotency
    let idem_key = headers
        .get("Idempotency-Key")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    if let Some(ref k) = idem_key {
        if let Ok(Some(existing)) = state.registry.check_idempotent_conflict(k, "pool.scale") {
            return Ok(accepted(&existing.id, crate::schemas::to_task_dict(&existing)));
        }
    }

    // Validate target synchronously
    let _ = body.as_object();
    let target = crate::schemas::validate_scale_target(Some(&body))
        .map_err(|e| ApiError::validation(e.0, &rid))?;

    // Create async task to converge pool toward target
    let task = state.registry.create("pool.scale");
    if let Some(k) = idem_key {
        state.registry.set_idempotent(k, task.id.clone());
    }
    let task_id = task.id.clone();

    let st = state.clone();
    state.registry.run_inline(task_id.clone(), move || async move {
        st.pool.set_target(target);
        st.pool
            .ensure_count(&st.docker, "warpgate:local", "egressd-net", "egressd-")
            .await;
        let summary = st.pool.summary(false).await;
        Ok(Some(json!({
            "target": summary.target,
            "pool_size": summary.pool_size,
            "healthy": summary.healthy,
            "degraded": summary.degraded,
        })))
    })
    .await;

    Ok(accepted(&task_id, crate::schemas::to_task_dict(&state.registry.get(&task_id).unwrap())))
}

#[derive(serde::Deserialize)]
struct ProxiesQuery {
    healthy: Option<String>,
}

async fn list_proxies(
    State(state): State<AppState>,
    Query(q): Query<ProxiesQuery>,
    headers: HeaderMap,
) -> ApiResult {
    let rid = new_request_id();
    check_auth(&state, &headers, "client")?;
    let filter = match q.healthy.as_deref() {
        None => None,
        Some("true") => Some(true),
        Some("false") => Some(false),
        Some(_) => {
            return Err(ApiError::validation(
                "healthy must be 'true' or 'false'",
                &rid,
            ))
        }
    };
    let snap = state.pool.snapshot().await;
    let filtered: Vec<&Endpoint> = snap
        .iter()
        .filter(|e| filter.map(|f| e.healthy == f).unwrap_or(true))
        .collect();
    let arr: Vec<Value> = filtered.iter().map(|e| crate::schemas::to_proxy_dict(e)).collect();
    Ok(ok_json(Value::Array(arr), &rid))
}

async fn create_proxy(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Option<Json<Value>>,
) -> ApiResult {
    let rid = new_request_id();
    check_auth(&state, &headers, "client")?;
    let raw_body = body.as_ref().map(|Json(v)| v.clone());

    let idem_key = headers
        .get("Idempotency-Key")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    // Idempotent replay — return existing task before any other checks
    if let Some(ref k) = idem_key {
        match state.registry.check_idempotent_conflict(k, "proxy.create") {
            Ok(Some(existing)) => {
                return Ok(accepted(&existing.id, crate::schemas::to_task_dict(&existing)));
            }
            Err(e) => {
                let code: &'static str = match e.code.as_str() {
                    "IDEMPOTENCY_CONFLICT" => "IDEMPOTENCY_CONFLICT",
                    _ => "CONFLICT",
                };
                return Err(ApiError::conflict(code, e.message, &rid));
            }
            Ok(None) => {}
        }
    }

    // Capacity + duplicate checks sync
    {
        let size = state.pool.pool_size().await;
        if size >= MAX_POOL {
            return Err(ApiError::conflict(
                "POOL_AT_CAPACITY",
                format!("pool at max size ({MAX_POOL})"),
                &rid,
            ));
        }
    }
    if let Some(Json(ref b)) = body {
        if let Some(name) = crate::schemas::validate_proxy_name(Some(b))
            .map_err(|e| ApiError::validation(e.0, &rid))?
        {
            let exists = state
                .pool
                .snapshot()
                .await
                .iter()
                .any(|e| e.name == name);
            if exists {
                return Err(ApiError::conflict(
                    "PROXY_EXISTS",
                    format!("proxy {name} already exists"),
                    &rid,
                ));
            }
        }
    }

    let explicit_name = raw_body
        .as_ref()
        .and_then(|b| crate::schemas::validate_proxy_name(Some(b)).ok().flatten());

    let task = state.registry.create("proxy.create");
    if let Some(k) = idem_key {
        state.registry.set_idempotent(k, task.id.clone());
    }
    let task_id = task.id.clone();

    let st = state.clone();
    state.registry.run_inline(task_id.clone(), move || async move {
        // Create container via FakeDocker (real DockerClient in prod)
        let name = explicit_name.unwrap_or_else(|| format!("egressd-{:08x}", rand::random::<u32>()));
        let cid = st.docker.create(&name, "warpgate:local", "egressd-net");
        match st.pool.add_proxy(name, cid).await {
            Ok(ep) => Ok(Some(crate::schemas::to_proxy_dict(&ep))),
            Err(e) => Err(TaskError::new("CREATE_FAILED", e)),
        }
    })
    .await;

    Ok(accepted(&task_id, crate::schemas::to_task_dict(&state.registry.get(&task_id).unwrap())))
}

async fn get_proxy(
    State(state): State<AppState>,
    Path(proxy_id): Path<String>,
    headers: HeaderMap,
) -> ApiResult {
    let rid = new_request_id();
    check_auth(&state, &headers, "client")?;
    let ep = state
        .pool
        .snapshot()
        .await
        .into_iter()
        .find(|e| e.name == proxy_id)
        .ok_or_else(|| ApiError::not_found("PROXY_NOT_FOUND", format!("proxy {proxy_id} not found"), &rid))?;
    Ok(ok_json(crate::schemas::to_proxy_dict(&ep), &rid))
}

async fn delete_proxy(
    State(state): State<AppState>,
    Path(proxy_id): Path<String>,
    headers: HeaderMap,
) -> ApiResult {
    let rid = new_request_id();
    check_auth(&state, &headers, "client")?;
    let exists = state
        .pool
        .snapshot()
        .await
        .iter()
        .any(|e| e.name == proxy_id);
    if !exists {
        return Err(ApiError::not_found(
            "PROXY_NOT_FOUND",
            format!("proxy {proxy_id} not found"),
            &rid,
        ));
    }
    let task = state.registry.create("proxy.remove");
    let task_id = task.id.clone();
    let st = state.clone();
    let name = proxy_id.clone();
    state.registry.run_inline(task_id.clone(), move || async move {
        if !st.pool.remove_proxy(&st.docker, &name).await {
            return Err(TaskError::new(
                "PROXY_NOT_FOUND",
                format!("proxy {name} not found"),
            ));
        }
        Ok(Some(json!({"status":"removed","id":name})))
    })
    .await;
    Ok(accepted(&task_id, crate::schemas::to_task_dict(&state.registry.get(&task_id).unwrap())))
}

async fn rotate_proxy(
    State(state): State<AppState>,
    Path(proxy_id): Path<String>,
    headers: HeaderMap,
) -> ApiResult {
    let rid = new_request_id();
    check_auth(&state, &headers, "client")?;
    let exists = state.pool.snapshot().await.iter().any(|e| e.name == proxy_id);
    if !exists {
        return Err(ApiError::not_found(
            "PROXY_NOT_FOUND",
            format!("proxy {proxy_id} not found"),
            &rid,
        ));
    }
    let task = state.registry.create("proxy.rotate");
    let task_id = task.id.clone();
    let name = proxy_id.clone();
    let st = state.clone();
    state.registry.run_inline(task_id.clone(), move || async move {
        // MVP rotation = restart container (recreate same name)
        st.pool.set_busy(&name, true).await;
        let _ = st.docker.remove(&name);
        let _cid = st.docker.create(&name, "warpgate:local", "egressd-net");
        st.pool.set_busy(&name, false).await;
        Ok(Some(json!({"rotated": true})))
    })
    .await;
    Ok(accepted(&task_id, crate::schemas::to_task_dict(&state.registry.get(&task_id).unwrap())))
}

async fn restart_proxy(
    State(state): State<AppState>,
    Path(proxy_id): Path<String>,
    headers: HeaderMap,
) -> ApiResult {
    let rid = new_request_id();
    check_auth(&state, &headers, "client")?;
    let exists = state.pool.snapshot().await.iter().any(|e| e.name == proxy_id);
    if !exists {
        return Err(ApiError::not_found(
            "PROXY_NOT_FOUND",
            format!("proxy {proxy_id} not found"),
            &rid,
        ));
    }
    let task = state.registry.create("proxy.restart");
    let task_id = task.id.clone();
    let name = proxy_id.clone();
    let st = state.clone();
    state.registry.run_inline(task_id.clone(), move || async move {
        st.pool.set_busy(&name, true).await;
        let old_cid = st.docker.by_name.lock().unwrap().get(&name).cloned();
        if let Some(cid) = old_cid {
            let _ = st.docker.remove(&cid);
        }
        let new_cid = st.docker.create(&name, "warpgate:local", "egressd-net");
        st.pool.set_busy(&name, false).await;
        Ok(Some(json!({"restarted": true, "container_id": new_cid})))
    })
    .await;
    Ok(accepted(&task_id, crate::schemas::to_task_dict(&state.registry.get(&task_id).unwrap())))
}

async fn rotate_proxies_bulk(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Option<Json<Value>>,
) -> ApiResult {
    let rid = new_request_id();
    check_auth(&state, &headers, "client")?;
    let scope_raw = body.as_ref().map(|Json(v)| v.clone());
    let scope = crate::schemas::validate_rotate_scope(scope_raw.as_ref())
        .map_err(|e| ApiError::validation(e.0, &rid))?;

    let task = state.registry.create("proxy.rotate_bulk");
    let task_id = task.id.clone();
    let st = state.clone();
    state.registry.run_inline(task_id.clone(), move || async move {
        let snap = st.pool.snapshot().await;
        let names: Vec<String> = snap
            .into_iter()
            .filter(|e| scope == "all" || !e.healthy)
            .map(|e| e.name)
            .collect();
        let mut rotated = vec![];
        for n in names {
            st.pool.set_busy(&n, true).await;
            let _ = st.docker.remove(&n);
            let _ = st.docker.create(&n, "warpgate:local", "egressd-net");
            st.pool.set_busy(&n, false).await;
            rotated.push(n);
        }
        Ok(Some(json!({"rotated": rotated, "failed": [], "count": rotated.len()})))
    })
    .await;
    Ok(accepted(&task_id, crate::schemas::to_task_dict(&state.registry.get(&task_id).unwrap())))
}

async fn get_task(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
    headers: HeaderMap,
) -> ApiResult {
    let rid = new_request_id();
    check_auth(&state, &headers, "client")?;
    let t = state
        .registry
        .get(&task_id)
        .ok_or_else(|| ApiError::not_found("TASK_NOT_FOUND", format!("task {task_id} not found"), &rid))?;
    let mut resp = ok_json(crate::schemas::to_task_dict(&t), &rid);
    if matches!(t.status, TaskStatus::Pending | TaskStatus::Running) {
        resp.headers_mut()
            .insert("Retry-After", HeaderValue::from_static("1"));
    }
    Ok(resp)
}

async fn openapi_spec(State(_): State<AppState>) -> Response {
    let yaml = include_str!("../../../static/openapi.yaml");
    (
        [(axum::http::header::CONTENT_TYPE, "application/yaml")],
        yaml.to_string(),
    )
        .into_response()
}

async fn docs(State(_): State<AppState>) -> Response {
    const SWAGGER_HTML: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8"/>
  <title>egressd API</title>
  <link rel="stylesheet" href="https://unpkg.com/swagger-ui-dist@5/swagger-ui.css"/>
</head>
<body>
  <div id="swagger-ui"></div>
  <script src="https://unpkg.com/swagger-ui-dist@5/swagger-ui-bundle.js"></script>
  <script>window.onload=function(){window.ui=SwaggerUIBundle({url:"/v1/openapi.yaml",dom_id:"#swagger-ui"});};</script>
</body>
</html>"##;
    (
        [(axum::http::header::CONTENT_TYPE, "text/html")],
        SWAGGER_HTML.to_string(),
    )
        .into_response()
}

async fn not_found_fallback() -> Response {
    ApiError::new(StatusCode::NOT_FOUND, "NOT_FOUND", "resource not found", "").into_response()
}

async fn method_not_allowed() -> Response {
    ApiError::new(StatusCode::METHOD_NOT_ALLOWED, "METHOD_NOT_ALLOWED", "method not allowed", "").into_response()
}

// ── Router ───────────────────────────────────────────────────────────────

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/pool", get(get_pool).patch(patch_pool))
        .route(
            "/v1/proxies",
            get(list_proxies).post(create_proxy),
        )
        .route("/v1/proxies/rotate", post(rotate_proxies_bulk))
        .route("/v1/proxies/{proxy_id}", get(get_proxy).delete(delete_proxy))
        .route("/v1/proxies/{proxy_id}/rotate", post(rotate_proxy))
        .route("/v1/proxies/{proxy_id}/restart", post(restart_proxy))
        .route("/v1/tasks/{task_id}", get(get_task))
        .route("/v1/openapi.yaml", get(openapi_spec))
        .route("/v1/docs", get(docs))
        .fallback(not_found_fallback)
        .method_not_allowed_fallback(method_not_allowed)
        .with_state(state)
}

/// Run the server (bind addr from env or default `0.0.0.0:9090`).
pub async fn run(state: AppState) -> anyhow::Result<()> {
    let port: u16 = std::env::var("EGRESSD_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(9090);
    let app = router(state);
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    tracing::info!(port, "egressd listening");
    axum::serve(listener, app).await?;
    Ok(())
}
