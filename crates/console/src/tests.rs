//! Unit tests for the console crate: the npm-install decision logic (compiled from the build
//! script's `install.rs`, the single source of truth, via `#[path]`, because build scripts cannot
//! import their own crate) and the shared asset-serving layer. The `#[path]` attribute is relative
//! to this file's directory, so `../build/install.rs` reaches the build script.

use std::borrow::Cow;

use axum::{
    http::{StatusCode, Uri, header},
    response::Response,
};

use crate::{asset_or_index, serve, serve_embedded};

/// The npm-install decision logic, tested as compiled into this module.
#[path = "../build/install.rs"]
mod install;

/// A synthetic embedded file, so the asset-layer tests need no real built bundle (nor Node):
/// `EmbeddedFile` and `Metadata` are fully public in `rust-embed-utils`.
fn file(kind: &str, body: &'static str) -> rust_embed::EmbeddedFile {
    rust_embed::EmbeddedFile {
        data: Cow::Borrowed(body.as_bytes()),
        metadata: rust_embed::Metadata::__rust_embed_new(
            [0; 32],
            None,
            None,
            match kind {
                "html" => "text/html; charset=utf-8",
                _ => "application/javascript",
            },
        ),
    }
}

async fn body_of(response: Response) -> String {
    String::from_utf8(
        axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap()
}

/// The HTML shell gets the mode injected; non-HTML bytes pass through untouched.
#[tokio::test]
async fn serve_injects_the_mode_into_html_and_passes_bytes_through() {
    let html = file("html", "<html>__ZUIHITSU_APP_MODE__</html>");
    let response = serve(html, "eval");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/html; charset=utf-8"
    );
    assert_eq!(body_of(response).await, "<html>eval</html>");

    let js = file("js", "window.__ZUIHITSU_APP_MODE__");
    let response = serve(js, "eval");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "application/javascript"
    );
    assert_eq!(body_of(response).await, "window.__ZUIHITSU_APP_MODE__");
}

/// The `asset_or_index` fallback: an unknown asset resolves to `index.html`, the root to
/// `index.html` directly.
#[test]
fn asset_or_index_falls_back_to_the_index_shell() {
    let root = asset_or_index("index.html").expect("the real embed ships index.html");
    assert!(root.data.starts_with(b"<!doctype html>"));

    // An unknown path resolves to the same embed.
    let missing = asset_or_index("definitely/not/an/asset.js").expect("falls back to index.html");
    assert_eq!(missing.data, root.data);
}

/// `serve_embedded` maps the root and unknown client-side routes onto `index.html` (this crate
/// always embeds a real folder, so the fallback exists in this test through the real embed).
#[tokio::test]
async fn serve_embedded_answers_root_and_client_routes() {
    for path in ["/", "/some/client/route"] {
        let response = serve_embedded(&Uri::from_static(path), "agent");
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            response.headers()[header::CONTENT_TYPE]
                .to_str()
                .unwrap()
                .starts_with("text/html")
        );
        let body = body_of(response).await;
        assert!(
            body.contains("agent"),
            "the agent bundle should mention its mode"
        );
    }
}
