//! The console build pipeline and shared embedding API.
//!
//! This crate owns the entire frontend build pipeline (ts-rs type export, settings metadata, wasm
//! build, wasm-bindgen, wasm-opt, npm ci, and the Vite production build). `build.rs` runs it
//! unconditionally whenever the crate is compiled. The crate has no Cargo features: depending on it
//! means you want the console embedded.
//!
//! The Vite build (with `VITE_EMBEDDED=true`) writes into this crate's `dist-embedded/` directory
//! (gitignored), which [`Console`] embeds at compile time. The embedded `index.html` ships with a
//! `__ZUIHITSU_APP_MODE__` template token that [`render_embedded`] replaces at serve time, so the
//! one shared bundle boots into whichever host serves it: `agent` mode from the agent binary,
//! `eval` mode from the eval binary. Consumers resolve a path via [`asset_or_index`] and turn the
//! rendered bytes into their own HTTP response (see `zuihitsu`'s `src/http_server/console.rs` and
//! `zuihitsu-eval`'s `src/serve.rs`). The crate is content-only: it does not depend on an HTTP
//! framework, so serving stays the consumer's job.

use zuihitsu_frontend_types::AppMode;

/// The web console, built into the binary at compile time (see `build.rs`). The embedded build
/// lands in this crate's own `dist-embedded` dir, so a plain `npm run build` for the dev checks
/// never swaps in the standalone (non-embedded) bytes.
#[derive(rust_embed::RustEmbed)]
#[folder = "dist-embedded"]
pub struct Console;

/// Serve a console asset by path, falling back to `index.html` for client-side routes so a deep
/// link or a refresh lands in the app rather than on a 404.
pub fn asset_or_index(path: &str) -> Option<rust_embed::EmbeddedFile> {
    Console::get(path).or_else(|| Console::get("index.html"))
}

/// Render an embedded console asset for the given mode: the HTML shell gets `__ZUIHITSU_APP_MODE__`
/// replaced with the mode's token value, and non-HTML assets pass through byte-for-byte. Returns
/// the asset's mime type and rendered bytes; the consumer builds its HTTP response from them.
pub fn render_embedded(file: rust_embed::EmbeddedFile, mode: AppMode) -> (String, Vec<u8>) {
    let mime = file.metadata.mimetype().to_owned();
    if mime.starts_with("text/html") {
        let html =
            String::from_utf8_lossy(&file.data).replace("__ZUIHITSU_APP_MODE__", mode.as_str());
        (mime, html.into_bytes())
    } else {
        (mime, file.data.to_vec())
    }
}

#[cfg(test)]
mod tests;
