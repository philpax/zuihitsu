//! Maintenance passes: autonomous data-hygiene machinery that runs off the hot path.
//!
//! Three passes run on a timer, gated on activity: consolidation clusters semantically-overlapping
//! entries and synthesizes richer replacements (and dedups more-private copies against more-public
//! ones); canonicalize gives platform stubs readable named identities; link cleanup retracts entries
//! whose content is purely a description of a link that exists.
//!
//! All three passes drive their writes through the ordinary
//! [`MemoryBlock`](crate::memory::memory_block::MemoryBlock) write path under
//! [`Authority::Agent`](crate::memory::memory_block::Authority::Agent), so every write clears the same
//! guards a turn's writes do rather than appending raw events that bypass them. That tier carries
//! exactly the powers the passes need — cross-teller supersede for consolidation's cross-posture dedup,
//! the free-merge `same_as` assertion for canonicalize's mint-and-bind (an empty profile only), and
//! model-provenance-stamped retraction for link cleanup — with `self` writes blocked in every case. A
//! connector-maintained entry (a participant attribute a platform connector owns and keeps in step) is
//! excluded from every pass, so cleanup never fights the connector for an entry it will re-assert.
//!
//! The passes are cursor-resumed and idempotent, so an idle tick is cheap. A pass that finds nothing
//! to do still advances its cursor, so it does not re-scan the same window. The timer-driven passes
//! resume from their incremental cursor; the on-demand entry points sweep the whole log from the start,
//! since a fresh instance seeds every cursor to log-head at boot and an incremental cursor would make
//! the manual pass a no-op (see [`crate::instance`]'s pass facade).

pub mod canonicalize;
pub mod consolidation;
pub mod link_cleanup;

mod scheduling;

pub use scheduling::activity_gate;
