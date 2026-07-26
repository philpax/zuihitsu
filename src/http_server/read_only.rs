//! The read-only gate: one middleware layer per surface that refuses every mutating request when the
//! server is booted read-only (`--read-only`, or `[serving] read_only = true`).
//!
//! The gate is keyed on the request *method*, not on a list of handlers, because the method is the
//! property that actually decides it: on both surfaces every mutating route is a `POST` or a `PUT` and
//! every read route is a `GET`. A per-handler check would be a discipline to remember at each new
//! route, and forgetting it would leave a handler mutating an instance the operator booted for
//! inspection. Keyed on the method, a new route is gated by construction.
//!
//! Fail-closed: the *read* methods are the enumerated set and everything else is refused, so a method
//! nobody considered here is refused rather than let through. `HEAD` and `OPTIONS` are reads —
//! axum answers a `HEAD` from its `get` handler, so refusing it would break an ordinary probe.

use axum::{
    extract::{Request, State},
    http::Method,
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::http_server::{AppState, error::ApiError};

/// Refuse a mutating request with `409` when the server is booted read-only, and pass everything else
/// through. Layered onto each surface's sub-router beside its auth layer, inside it, so an
/// unauthorized request is rejected as unauthorized rather than told about the instance's boot mode.
pub(super) async fn refuse_mutations_when_read_only(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    if state.read_only && !is_read(request.method()) {
        return ApiError::ReadOnly.into_response();
    }
    next.run(request).await
}

/// Whether a method only reads. The enumerated side of the fail-closed split above.
fn is_read(method: &Method) -> bool {
    matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}
