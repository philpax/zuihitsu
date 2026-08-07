//! The console build pipeline and shared embedding API.
//!
//! This crate owns the entire frontend build pipeline (ts-rs type export, settings metadata, wasm
//! build, wasm-bindgen, wasm-opt, npm ci, and the Vite production build) — `build.rs` runs it
//! unconditionally whenever the crate is compiled. The crate has no Cargo features: depending on it
//! means you want the console embedded.
//!
//! The Vite build (with `VITE_EMBEDDED=true`) writes into this crate's `dist-embedded/` directory
//! (gitignored), which [`Console`] embeds at compile time. The embedded `index.html` ships with a
//! `__ZUIHITSU_APP_MODE__` template token that [`serve`] replaces at serve time, so the one shared
//! bundle boots into whichever host serves it — `agent` mode from the agent binary, `eval` mode
//! from the eval binary. Consumers resolve a path via [`asset_or_index`] and serve it via [`serve`]
//! (see `zuihitsu`'s `src/http_server/console.rs` and `zuihitsu-eval`'s `src/serve.rs`).

use axum::{
    http::{Uri, header},
    response::{IntoResponse, Response},
};

/// The web console, built into the binary at compile time (see `build.rs`). The embedded build
/// lands in this crate's own `dist-embedded` dir, so a plain `npm run build` for the dev checks
/// never swaps in the standalone (non-embedded) bytes under us.
#[derive(rust_embed::RustEmbed)]
#[folder = "dist-embedded"]
pub struct Console;

/// Serve a console asset by path, falling back to `index.html` for client-side routes so a deep
/// link or a refresh lands in the app rather than on a 404.
pub fn asset_or_index(path: &str) -> Option<rust_embed::EmbeddedFile> {
    Console::get(path).or_else(|| Console::get("index.html"))
}

/// Serve an embedded console asset, injecting the app mode into the HTML shell (replacing the
/// `__ZUIHITSU_APP_MODE__` placeholder `index.html` ships with) so the single shared bundle knows
/// which view to boot. Non-HTML assets are served byte-for-byte.
pub fn serve(file: rust_embed::EmbeddedFile, mode: &str) -> Response {
    let mime = file.metadata.mimetype().to_owned();
    if mime.starts_with("text/html") {
        let html = String::from_utf8_lossy(&file.data).replace("__ZUIHITSU_APP_MODE__", mode);
        ([(header::CONTENT_TYPE, mime)], html).into_response()
    } else {
        ([(header::CONTENT_TYPE, mime)], file.data).into_response()
    }
}

/// Resolve a request URI to an asset and serve it in the given mode: the asset itself, or
/// `index.html` for a client-side route (and for the bare root path), or `404` when the fallback
/// itself is absent (a console-less build of this crate cannot happen — `build.rs` panics — so a
/// missing `index.html` means the embed folder was tampered with after the build).
pub fn serve_embedded(uri: &Uri, mode: &str) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    match asset_or_index(path) {
        Some(file) => serve(file, mode),
        None => (
            axum::http::StatusCode::NOT_FOUND,
            "the web console is not built into this binary\n",
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests;
