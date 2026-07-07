use super::*;
fn keyed_app(control: &[&str], platform: &[&str]) -> axum::Router {
    let server =
        Arc::new(Server::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap());
    let keys = |k: &[&str]| -> Arc<[String]> { k.iter().map(|s| s.to_string()).collect() };
    router(AppState {
        control_keys: keys(control),
        platform_keys: keys(platform),
        ..test_state(server)
    })
}

/// A GET request from `peer`, optionally bearing `key`.
fn get(peer: ConnectInfo<SocketAddr>, uri: &str, key: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().extension(peer).uri(uri);
    if let Some(key) = key {
        builder = builder.header("authorization", format!("Bearer {key}"));
    }
    builder.body(Body::empty()).unwrap()
}

#[tokio::test]
async fn a_remote_peer_without_a_valid_key_is_rejected() {
    let app = keyed_app(&["op-key"], &["pf-key"]);
    // No Authorization header → 401, on both surfaces.
    let response = app
        .clone()
        .oneshot(get(remote(), "/control/genesis", None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .extension(remote())
                .method("POST")
                .uri("/platform/message")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    // A wrong key → 401.
    let response = app
        .oneshot(get(remote(), "/control/genesis", Some("nope")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_valid_key_authorizes_only_its_own_surface() {
    let app = keyed_app(&["op-key"], &["pf-key"]);
    // The control key opens a control route.
    let response = app
        .clone()
        .oneshot(get(remote(), "/control/genesis", Some("op-key")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    // ...but the same key does NOT open a platform route — the surfaces are isolated.
    let response = app
        .oneshot(
            Request::builder()
                .extension(remote())
                .method("POST")
                .uri("/platform/message")
                .header("authorization", "Bearer op-key")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_loopback_peer_is_trusted_without_a_key() {
    // Even with keys configured, a loopback peer needs none — the local CLI keeps working.
    let app = keyed_app(&["op-key"], &["pf-key"]);
    let response = app
        .oneshot(get(loopback(), "/control/genesis", None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn an_empty_key_list_is_fail_closed_for_remote_peers() {
    // No keys configured + a remote peer → always rejected, so a wide bind with no keys is a
    // silent lockout, never a silent exposure.
    let app = keyed_app(&[], &[]);
    let response = app
        .oneshot(get(remote(), "/control/genesis", None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
