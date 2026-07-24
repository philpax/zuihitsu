use crate::http_server::tests::*;
fn keyed_app(control: &[&str], platform_connectors: &[&str]) -> axum::Router {
    let server =
        Arc::new(Server::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap());
    let control_keys: Arc<[String]> = control.iter().map(|s| s.to_string()).collect();
    // Each connector key registers under a distinct platform — the platform scopes the request; the auth
    // tests only assert the key is accepted, so a generated platform suffices.
    let platform_connectors: Arc<[(String, String)]> = platform_connectors
        .iter()
        .enumerate()
        .map(|(index, key)| (format!("connector{index}"), key.to_string()))
        .collect();
    router(AppState {
        control_keys,
        platform_connectors,
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
                .uri("/platform/messages")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    // The auth layer is method-agnostic: a GET platform route is guarded the same as the POSTs.
    let response = app
        .clone()
        .oneshot(get(remote(), "/platform/self", None))
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
                .uri("/platform/messages")
                .header("authorization", "Bearer op-key")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_connector_key_scopes_a_remote_request_onto_the_platform() {
    // A remote peer bearing a registered connector's key clears the participant surface — and needs no
    // platform in the body, because the key is the scope. Without a model configured the turn answers
    // 503, which proves it passed the auth-and-scope layer rather than being rejected 401.
    let app = keyed_app(&[], &["pf-key"]);
    let response = app
        .oneshot(
            Request::builder()
                .extension(remote())
                .method("POST")
                .uri("/platform/messages")
                .header("authorization", "Bearer pf-key")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"scope_path":"general","messages":[{"sender":"dave","text":"hi"}],"present":["dave"]}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
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
