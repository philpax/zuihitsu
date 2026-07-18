//! Per-surface bearer-key middleware (spec §Trust model). The operator surface trusts a loopback peer
//! and requires a control key from a remote one. The participant surface instead *scopes* every request
//! to a connector: a loopback request is the operator's own `direct` interface, and a remote request
//! must present a connector's key, which resolves to exactly that connector's platform. A control key
//! never authorizes `/platform` and vice versa, and no request carries a platform to spoof — the key is
//! the one source of truth for which platform a connector acts on.

use std::net::SocketAddr;

use axum::{
    extract::{ConnectInfo, Request, State},
    http::{StatusCode, header::AUTHORIZATION},
    middleware::Next,
    response::{IntoResponse, Response},
};
use sha2::{Digest, Sha256};

use zuihitsu::ids::DIRECT_PLATFORM;

use crate::http_server::AppState;

/// The platform a `/platform/*` request is authenticated for: the key resolves to the connector's
/// platform, every operation is scoped to it, and writes are attributed to its connector. Inserted into
/// the request by [`require_platform_key`] and read by each participant handler; the handler never
/// trusts a platform from the body, because there is none.
#[derive(Clone, Debug)]
pub(super) struct PlatformConnectorScope {
    pub platform: String,
}

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

/// Participant-surface auth and scoping: resolve the request to exactly one connector, and stamp its
/// [`PlatformConnectorScope`] onto the request for the handler to act under. The key is checked *first*, so a
/// connector running on the same host as the server (a bot on `localhost`, the common deployment) is
/// still scoped to its own platform by its key rather than mistaken for the operator's console. A
/// request bearing a registered connector's key is scoped to that connector, wherever it connects from;
/// a request bearing no key falls back to the loopback rule — the operator's own console, scoped to the
/// reserved `direct` platform, and rejected from a remote peer; and a request bearing an *unrecognized*
/// key is rejected outright (a misconfigured connector should fail loudly, never silently act as
/// `direct`). Fail-closed: with no connectors configured, only a keyless loopback peer (as `direct`)
/// reaches the surface.
pub(super) async fn require_platform_key(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    mut request: Request,
    next: Next,
) -> Response {
    let scope = match presented_key(&request) {
        Some(key) => match resolve_platform_connector(&state.platform_connectors, key) {
            Some(platform) => PlatformConnectorScope { platform },
            None => return StatusCode::UNAUTHORIZED.into_response(),
        },
        None if peer.ip().is_loopback() => PlatformConnectorScope {
            platform: DIRECT_PLATFORM.to_owned(),
        },
        None => return StatusCode::UNAUTHORIZED.into_response(),
    };
    request.extensions_mut().insert(scope);
    next.run(request).await
}

/// The bearer key a request presents, or `None` if it carries no `Authorization: Bearer` header.
fn presented_key(request: &Request) -> Option<&str> {
    request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
}

/// Resolve a presented bearer key to the platform the connector it registers serves, or `None` if it
/// matches none. Compares fixed-width SHA-256 digests and scans the whole registry unconditionally (no
/// early return), so neither the key's length nor the matching connector's position leaks through timing
/// — the same discipline as [`key_is_valid`].
fn resolve_platform_connector(
    platform_connectors: &[(String, String)],
    presented: &str,
) -> Option<String> {
    let presented = Sha256::digest(presented.as_bytes());
    let mut matched = None;
    for (platform, key) in platform_connectors {
        if presented == Sha256::digest(key.as_bytes()) {
            matched = Some(platform.clone());
        }
    }
    matched
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
