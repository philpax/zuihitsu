//! Main-crate types: the wire-contract types that originated in the main `zuihitsu` crate.

use serde::Serialize;

/// The circuit's observable state, for the operator health surface and the state gauge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum CircuitState {
    Closed,
    HalfOpen,
    Open,
}

/// The model transport's health, as the operator surface reports it: the circuit state, the
/// consecutive transient-failure count, the last failure's cause (kept across recovery, so an
/// operator can still read what went wrong), and — while open — how long until the half-open probe.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BackendHealth {
    pub circuit: CircuitState,
    pub consecutive_failures: u32,
    pub last_failure: Option<String>,
    pub open_remaining_ms: Option<u64>,
}
