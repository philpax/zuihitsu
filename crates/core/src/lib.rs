//! Zuihitsu core — the wasm-compatible heart of the agent.
//!
//! This crate holds the event-log wire types and the pure projection logic that turns an event log
//! into queryable state: the same fold the live agent runs, carved out so it can compile to
//! wasm32-unknown-unknown and drive the console as a materializing read replica (see
//! `console/PLAN.md`). It carries none of the host-only machinery — no Lua VM, no async runtime, no
//! HTTP, no model client, no file-backed SQLite — so the boundary is "what a browser can run."
//!
//! The main `zuihitsu` crate depends on this and re-exports every module, so the rest of the
//! codebase continues to reach these types at their familiar `crate::*` paths.

pub mod db;
pub mod event;
pub mod graph;
pub mod ids;
pub mod model;
pub mod settings;
pub mod store;
pub mod time;
pub mod visibility;
pub mod vocabulary;
