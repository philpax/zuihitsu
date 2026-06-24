//! Per-surface bearer-key middleware (spec §Trust model). A loopback peer is trusted without a key; a
//! remote peer must present a valid key for the surface it is reaching. The two surfaces carry
//! independent key lists, so a control key never authorizes `/platform` and vice versa.

use std::net::SocketAddr;

use axum::{
    extract::{ConnectInfo, Request, State},
    http::{StatusCode, header::AUTHORIZATION},
    middleware::Next,
    response::{IntoResponse, Response},
};
use sha2::{Digest, Sha256};

use super::AppState;

/// Operator-surface auth: a loopback peer passes without a key; a remote peer must present a valid
/// control key (spec §Trust model).
pub(super) async fn require_control_key(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    request: Request,
    next: Next,
) -> Response {
    authorize(&state.control_keys, peer, request, next).await
}

/// Participant-surface auth: the same rule against the platform key list.
pub(super) async fn require_platform_key(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    request: Request,
    next: Next,
) -> Response {
    authorize(&state.platform_keys, peer, request, next).await
}

/// Trust a loopback peer; require a valid bearer key from every remote peer. Fail-closed — an empty
/// key list rejects every remote peer, so a routable bind with no keys is a silent lockout rather than
/// a silent exposure (spec §Trust model). A reverse proxy would make every peer appear loopback, so
/// this must not be fronted by one without re-checking auth.
async fn authorize(keys: &[String], peer: SocketAddr, request: Request, next: Next) -> Response {
    if peer.ip().is_loopback() {
        return next.run(request).await;
    }
    let presented = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        // The scheme match is deliberately case-sensitive: the clients are ours, not arbitrary agents.
        .and_then(|value| value.strip_prefix("Bearer "));
    match presented {
        Some(key) if key_is_valid(key, keys) => next.run(request).await,
        _ => StatusCode::UNAUTHORIZED.into_response(),
    }
}

/// Whether `presented` matches any configured key. Compares fixed-width SHA-256 digests rather than the
/// raw strings, so the comparison time does not depend on the key's length or a shared prefix (a plain
/// `==` on the strings would leak both through early exit); the whole list is scanned unconditionally
/// (`|=`, not an early `return`), so the number of configured keys and the matching position do not
/// leak through timing either.
fn key_is_valid(presented: &str, keys: &[String]) -> bool {
    let presented = Sha256::digest(presented.as_bytes());
    let mut matched = false;
    for key in keys {
        matched |= presented == Sha256::digest(key.as_bytes());
    }
    matched
}
