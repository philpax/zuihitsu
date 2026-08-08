use crate::http_server::tests::*;
use axum::{
    body::Body,
    http::{Request, header},
};
use tower::ServiceExt;

async fn get(path: &str) -> (axum::http::StatusCode, String, String) {
    let server =
        Arc::new(Server::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap());
    let app = router(test_state(server));
    let response = app
        .oneshot(
            Request::builder()
                .extension(loopback())
                .uri(path)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let body = String::from_utf8(
        axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    (status, content_type, body)
}

/// The console fallback answers the root and client-side routes with the HTML shell.
#[tokio::test]
async fn console_serves_the_root_and_client_routes() {
    for path in ["/", "/some/client/route"] {
        let (status, content_type, body) = get(path).await;
        assert_eq!(status, StatusCode::OK);
        assert!(content_type.starts_with("text/html"), "{content_type}");
        assert!(!body.is_empty());
    }
}

/// The two CI jobs cover the two builds. The rust job (no frontend toolchain,
/// `--no-default-features`) exercises the placeholder path: the served body is exactly the
/// root-owned placeholder page, which carries no mode token. The console job's
/// `cargo test -p zuihitsu http_server::tests::console` step exercises the real embedded bundle,
/// with the `__ZUIHITSU_APP_MODE__` token replaced by `agent`.
#[tokio::test]
async fn console_serves_the_placeholder_or_the_agent_mode_shell() {
    let (_status, content_type, body) = get("/").await;
    assert!(content_type.starts_with("text/html"), "{content_type}");
    if body == PLACEHOLDER_BODY {
        // The placeholder build serves the root's own page byte-for-byte.
    } else {
        // The real embedded bundle replaces the mode token with the agent mode.
        assert!(
            !body.contains("__ZUIHITSU_APP_MODE__"),
            "the served shell must have the mode token replaced"
        );
        assert!(
            body.contains("agent"),
            "the agent bundle should mention its mode"
        );
    }
}

/// The placeholder page the `--no-default-features` build serves, byte for byte.
const PLACEHOLDER_BODY: &str = include_str!("../placeholder.html");
