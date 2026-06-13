//! The memory subsystem: the write path and the read paths over the materialized graph.
//!
//! A [`memory_block`] is the transactional unit that turns agent intent into events; [`visibility`]
//! decides who may see a given memory; [`search`] and [`brief`] are the two retrieval surfaces (a
//! ranked query and a composed context block); and [`identity`] resolves conversations and
//! participants to their canonical memories.
//!
//! The visibility predicate is pure and graph-derived, so it lives in `zuihitsu-core` and is
//! re-exported here at its historical `memory::visibility` path.

pub mod brief;
pub mod identity;
pub mod memory_block;
pub mod scheduler;
pub mod search;

pub use zuihitsu_core::visibility;
