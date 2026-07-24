use crate::http_server::{AppState, router};
use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use std::{net::SocketAddr, sync::Arc};
use tower::ServiceExt;
use zuihitsu::{
    Completion, ManualClock, ScriptedModel, Server,
    metrics::{LATENCY_BUCKETS, describe},
    time::Timestamp,
};

mod api;
mod auth;
mod health;
mod metrics;

/// No configured keys — the existing tests run loopback, where keys are not consulted.
fn no_keys() -> Arc<[String]> {
    Vec::new().into()
}

/// No configured platform connectors — the existing platform tests run loopback, scoped to `direct`.
fn no_platform_connectors() -> Arc<[(String, String)]> {
    Vec::new().into()
}

/// An [`AppState`] with the fields every test shares (no model, no snapshot dir, no metrics, no
/// keys, a default config), so a test overrides only what its scenario exercises:
/// `router(AppState { model: Some(m), ..test_state(server) })`.
fn test_state(server: Arc<Server>) -> AppState {
    AppState {
        live: Arc::new(crate::http_server::stream::LiveEvents::start(&server)),
        shutdown: crate::http_server::console::ShutdownFlag::never(),
        server,
        model: None,
        backend: None,
        snapshot_dir: None,
        metrics: None,
        boot: std::time::Instant::now(),
        control_keys: no_keys(),
        platform_connectors: no_platform_connectors(),
        config: Arc::new(zuihitsu::EnvConfig::default()),
    }
}

/// A loopback peer extension to inject into a `oneshot` request (real `axum::serve` sets this from
/// the socket; `Request::builder()` does not). The auth middleware trusts a loopback peer, so the
/// existing assertions are unaffected.
fn loopback() -> ConnectInfo<SocketAddr> {
    ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 0)))
}

/// A non-loopback peer extension, for the auth tests — a remote peer must present a valid key.
fn remote() -> ConnectInfo<SocketAddr> {
    ConnectInfo(SocketAddr::from(([203, 0, 113, 1], 1234)))
}
