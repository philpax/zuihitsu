//! `memory.search` tests, grouped by concern: the search behaviour itself (recall, rendering, and
//! the hit-as-handle affordances), the fuzzy-write guard that refuses a write through a hit the query
//! did not name, and the block-scoped taint guard that closes the same launder across a fetched
//! handle. The shared crate-root imports and harness reach each submodule through `use super::*`.

pub(crate) use super::*;

mod behavior;
mod guard;
mod taint;
