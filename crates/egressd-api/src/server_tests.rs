use crate::server::{router, AppState};
use axum::body::Body;
use axum::http::Request;
use tower::ServiceExt;

fn test_state() -> AppState {
    AppState::new(0, None)
}

fn test_state_auth() -> AppState {
    AppState::new(0, Some("test-secret".to_string()))
}

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(resp.into_body(), 1_000_000).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn health_returns_200_and_shape() {
    let app = router(test_state());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/v1/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert!(resp.headers().contains_key("X-Request-ID"));
    let b = body_json(resp).await;
    assert_eq!(
        b,
        serde_json::json!({"status":"initializing","pool_size":0,"healthy":0,"degraded":0})
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn health_exempt_from_auth() {
    let app = router(test_state_auth());
    let resp = app
        .oneshot(Request::builder().uri("/v1/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

// ── Auth ────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auth_required() {
    let state = test_state_auth();
    let app = router(state.clone());
    let resp = app
        .oneshot(Request::builder().uri("/v1/pool").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    let app2 = router(state);
    let req = Request::builder()
        .uri("/v1/pool")
        .header("Authorization", "Bearer test-secret")
        .body(Body::empty())
        .unwrap();
    let resp = app2.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_id_on_errors() {
    let app = router(test_state_auth());
    let resp = app
        .oneshot(Request::builder().uri("/v1/pool").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
    assert!(resp.headers().contains_key("X-Request-ID"));
    let b = body_json(resp).await;
    assert_eq!(b["error"]["code"], "UNAUTHORIZED");
}

// ── Pool ────────────────────────────────────────────────────────────────

async fn seed_proxy(state: &AppState, name: &str, healthy: bool) {
    let cid = state.docker.create(name, "img", "net");
    state.pool.add_proxy(name.to_string(), cid).await.unwrap();
    if healthy {
        // Mark healthy via set_busy trick — direct pool mutation not exposed; use busy=false + healthy flag
        // For tests, we mutate through a helper on Pool (add set_healthy in Pool)
        state.pool.set_healthy(name, healthy).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_pool_summary_and_filters() {
    let state = test_state();
    seed_proxy(&state, "egressd-aaa", true).await;
    seed_proxy(&state, "egressd-bbb", false).await;
    let app = router(state.clone());

    let req = Request::builder().uri("/v1/pool").body(Body::empty()).unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);
    let b = body_json(resp).await;
    assert_eq!(b["pool_size"], 2);
    assert_eq!(b["healthy"], 1);
    assert_eq!(b["degraded"], 1);
    assert_eq!(b["proxies"].as_array().unwrap().len(), 2);

    let req = Request::builder()
        .uri("/v1/pool?include=none")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let b = body_json(resp).await;
    assert!(b.get("proxies").is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_pool_invalid_include_422() {
    let app = router(test_state());
    let req = Request::builder()
        .uri("/v1/pool?include=banana")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 422);
    let b = body_json(resp).await;
    assert_eq!(b["error"]["code"], "VALIDATION_ERROR");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scale_pool_async() {
    let state = test_state();
    let app = router(state.clone());
    let req = Request::builder()
        .method("PATCH")
        .uri("/v1/pool")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"target":2}"#))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 202);
    assert!(resp.headers()["Location"].to_str().unwrap().starts_with("/v1/tasks/"));
    let task = body_json(resp).await;
    let tid = task["id"].as_str().unwrap().to_string();

    // run_inline completes the task before the 202 is returned
    let done = state.registry.get(&tid).unwrap();
    assert!(matches!(done.status, egressd_core::task::TaskStatus::Succeeded));
    assert_eq!(state.pool.pool_size().await, 2);
    assert_eq!(state.pool.target(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scale_invalid_target_422() {
    let app = router(test_state());
    let req = Request::builder()
        .method("PATCH")
        .uri("/v1/pool")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"target":"many"}"#))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 422);
    let b = body_json(resp).await;
    assert_eq!(b["error"]["code"], "VALIDATION_ERROR");
}

// ── Proxies ─────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn create_proxy_async_bumps_target() {
    let state = test_state();
    let app = router(state.clone());
    let req = Request::builder()
        .method("POST")
        .uri("/v1/proxies")
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 202);
    assert!(resp.headers()["Location"].to_str().unwrap().starts_with("/v1/tasks/"));
    let task = body_json(resp).await;
    let tid = task["id"].as_str().unwrap().to_string();

    let done = state.registry.get(&tid).unwrap();
    assert!(matches!(done.status, egressd_core::task::TaskStatus::Succeeded), "{done:?}");
    assert_eq!(state.pool.pool_size().await, 1);
    assert_eq!(state.pool.target(), 1);
    let ep_name = state.pool.snapshot().await[0].name.clone();
    assert!(ep_name.starts_with("egressd-"));
    // Reconcile must not remove created proxy
    state.pool.ensure_count(&state.docker, "warpgate:local", "egressd-net", "egressd-").await;
    assert_eq!(state.pool.pool_size().await, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn create_named_duplicate_409() {
    let state = test_state();
    seed_proxy(&state, "egressd-custom", false).await;
    let app = router(state);
    let req = Request::builder()
        .method("POST")
        .uri("/v1/proxies")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"name":"egressd-custom"}"#))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 409);
    let b = body_json(resp).await;
    assert_eq!(b["error"]["code"], "PROXY_EXISTS");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn create_invalid_name_422() {
    let app = router(test_state());
    for payload in [r#"{"name":"has space"}"#, r#"{"name":12}"#, r#""not-an-object""#] {
        let req = Request::builder()
            .method("POST")
            .uri("/v1/proxies")
            .header("content-type", "application/json")
            .body(Body::from(payload))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 422, "payload {payload}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_proxies_filter() {
    let state = test_state();
    seed_proxy(&state, "egressd-aaa", true).await;
    seed_proxy(&state, "egressd-bbb", false).await;
    let app = router(state);

    for (q, want) in [
        ("/v1/proxies", vec!["egressd-aaa", "egressd-bbb"]),
        ("/v1/proxies?healthy=true", vec!["egressd-aaa"]),
        ("/v1/proxies?healthy=false", vec!["egressd-bbb"]),
    ] {
        let req = Request::builder().uri(q).body(Body::empty()).unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 200);
        let b = body_json(resp).await;
        let got: Vec<String> = b
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x["id"].as_str().unwrap().to_string())
            .collect();
        let mut got_sorted = got.clone();
        got_sorted.sort();
        let mut want_sorted = want.clone();
        want_sorted.sort();
        assert_eq!(got_sorted, want_sorted, "query {q}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_proxies_invalid_filter_422() {
    let app = router(test_state());
    let req = Request::builder()
        .uri("/v1/proxies?healthy=banana")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 422);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_single_and_404() {
    let state = test_state();
    seed_proxy(&state, "egressd-aaa", true).await;
    let app = router(state);

    let req = Request::builder()
        .uri("/v1/proxies/egressd-aaa")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);
    let b = body_json(resp).await;
    assert_eq!(b["id"], "egressd-aaa");

    let req = Request::builder()
        .uri("/v1/proxies/nope")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 404);
    let b = body_json(resp).await;
    assert_eq!(b["error"]["code"], "PROXY_NOT_FOUND");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delete_missing_404() {
    let app = router(test_state());
    let req = Request::builder()
        .method("DELETE")
        .uri("/v1/proxies/nope")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delete_existing_async() {
    let state = test_state();
    let docker_cid = state.docker.create("egressd-gone", "img", "net");
    state
        .pool
        .add_proxy("egressd-gone".to_string(), docker_cid)
        .await
        .unwrap();
    let app = router(state.clone());

    let req = Request::builder()
        .method("DELETE")
        .uri("/v1/proxies/egressd-gone")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 202);
    let task = body_json(resp).await;
    let tid = task["id"].as_str().unwrap().to_string();

    let done = state.registry.get(&tid).unwrap();
    assert!(matches!(done.status, egressd_core::task::TaskStatus::Succeeded), "{done:?}");
    assert_eq!(state.pool.pool_size().await, 0);
    assert_eq!(state.pool.target(), 0);
}

// ── Idempotency ─────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idempotency_dedup_create() {
    let state = test_state();
    let app = router(state.clone());

    let mk = || {
        Request::builder()
            .method("POST")
            .uri("/v1/proxies")
            .header("Idempotency-Key", "create-1")
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap()
    };
    let r1 = app.clone().oneshot(mk()).await.unwrap();
    let r2 = app.oneshot(mk()).await.unwrap();
    assert_eq!(r1.status(), 202);
    assert_eq!(r2.status(), 202);
    let t1 = body_json(r1).await;
    let t2 = body_json(r2).await;
    assert_eq!(t1["id"], t2["id"]);

    assert_eq!(state.pool.pool_size().await, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idempotency_conflict_cross_type() {
    let state = test_state();
    // Pre-register key with pool.scale type
    let t = state.registry.create("pool.scale");
    state.registry.set_idempotent("shared-key".into(), t.id.clone());

    let app = router(state);
    let req = Request::builder()
        .method("POST")
        .uri("/v1/proxies")
        .header("Idempotency-Key", "shared-key")
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 409);
    let b = body_json(resp).await;
    assert_eq!(b["error"]["code"], "IDEMPOTENCY_CONFLICT");
}

// ── Tasks ───────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn task_pending_retry_after() {
    let state = test_state();
    let task = state.registry.create("pool.scale");
    let app = router(state);
    let req = Request::builder()
        .uri(format!("/v1/tasks/{}", task.id))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers()["Retry-After"], "1");
    let b = body_json(resp).await;
    assert_eq!(b["status"], "pending");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn task_not_found() {
    let app = router(test_state());
    let req = Request::builder()
        .uri("/v1/tasks/does-not-exist")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 404);
    let b = body_json(resp).await;
    assert_eq!(b["error"]["code"], "TASK_NOT_FOUND");
}

// ── Spec / docs ─────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn openapi_and_docs() {
    let app = router(test_state());
    let req = Request::builder()
        .uri("/v1/openapi.yaml")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);
    let bytes = axum::body::to_bytes(resp.into_body(), 1_000_000).await.unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(text.contains("openapi"));

    let req = Request::builder().uri("/v1/docs").body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);
    let bytes = axum::body::to_bytes(resp.into_body(), 1_000_000).await.unwrap();
    let text = String::from_utf8_lossy(&bytes).to_lowercase();
    assert!(text.contains("swagger-ui"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unknown_route_envelope() {
    let app = router(test_state());
    let req = Request::builder()
        .uri("/v1/does-not-exist")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 404);
    let b = body_json(resp).await;
    assert_eq!(b["error"]["code"], "NOT_FOUND");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn submit_arc_executes_closure() {
    let state = test_state();
    let st = state.clone();
    let task = state.registry.create("proxy.create");
    let tid = task.id.clone();
    println!("submitting...");
    state.registry.submit_arc(tid.clone(), move || async move {
        println!("closure start");
        let name = "egressd-debug".to_string();
        let cid = st.docker.create(&name, "img", "net");
        println!("container created {cid}");
        let ep = st.pool.add_proxy(name, cid).await;
        println!("add_proxy done");
        Ok(Some(serde_json::json!({"name": ep.unwrap().name})))
    });
    for i in 0..100 {
        let t = state.registry.get(&tid).unwrap();
        if !matches!(t.status, egressd_core::task::TaskStatus::Pending | egressd_core::task::TaskStatus::Running) {
            println!("done after {i}");
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("still running");
}


#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn create_proxy_completes_via_http() {
    let state = test_state();
    let app = router(state.clone());
    let req = Request::builder()
        .method("POST")
        .uri("/v1/proxies")
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    println!("sending request...");
    let resp = app.oneshot(req).await.unwrap();
    println!("status {}", resp.status());
    assert_eq!(resp.status(), 202);
    let task = body_json(resp).await;
    let tid = task["id"].as_str().unwrap().to_string();
    for i in 0..200 {
        let t = state.registry.get(&tid).unwrap();
        if !matches!(t.status, egressd_core::task::TaskStatus::Pending | egressd_core::task::TaskStatus::Running) {
            println!("done after {i} polls");
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("still running: {:?}", state.registry.get(&tid));
}
