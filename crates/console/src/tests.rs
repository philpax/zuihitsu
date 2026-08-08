//! Unit tests for the console crate: the npm-install decision logic (compiled from the build
//! script's `install.rs`, the single source of truth, via `#[path]`, because build scripts cannot
//! import their own crate) and the shared asset layer. The `#[path]` attribute is relative to this
//! file's directory, so `../build/install.rs` reaches the build script.

use zuihitsu_frontend_types::AppMode;

use crate::{asset_or_index, render_embedded};

/// The npm-install decision logic, tested as compiled into this module.
#[path = "../build/install.rs"]
mod install;

/// A synthetic embedded file, so the asset-layer tests need no real built bundle (nor Node):
/// `EmbeddedFile` and `Metadata` are fully public in `rust-embed-utils`.
fn file(kind: &str, body: &'static str) -> rust_embed::EmbeddedFile {
    rust_embed::EmbeddedFile {
        data: std::borrow::Cow::Borrowed(body.as_bytes()),
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

/// The HTML shell gets the mode's token injected; non-HTML bytes pass through untouched.
#[test]
fn render_embedded_injects_the_mode_into_html_and_passes_bytes_through() {
    let html = file("html", "<html>__ZUIHITSU_APP_MODE__</html>");
    let (mime, bytes) = render_embedded(html, AppMode::Eval);
    assert_eq!(mime, "text/html; charset=utf-8");
    assert_eq!(bytes, b"<html>eval</html>");

    let js = file("js", "window.__ZUIHITSU_APP_MODE__");
    let (mime, bytes) = render_embedded(js, AppMode::Eval);
    assert_eq!(mime, "application/javascript");
    assert_eq!(bytes, b"window.__ZUIHITSU_APP_MODE__");
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

/// The root and client-side routes resolve to the HTML shell, rendered for the agent mode.
#[test]
fn asset_or_index_answers_the_root_and_client_routes() {
    for path in ["/", "/some/client/route"] {
        let path = path.trim_start_matches('/');
        let path = if path.is_empty() { "index.html" } else { path };
        let file = asset_or_index(path).expect("falls back to index.html");
        let (mime, bytes) = render_embedded(file, AppMode::Agent);
        assert!(mime.starts_with("text/html"));
        let body = String::from_utf8(bytes).unwrap();
        assert!(
            body.contains("agent"),
            "the agent bundle should mention its mode"
        );
    }
}
