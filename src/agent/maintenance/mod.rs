//! Maintenance passes: autonomous data-hygiene machinery that runs off the hot path.
//!
//! Three passes run on a timer, gated on activity: consolidation clusters semantically-overlapping
//! entries and synthesizes richer replacements; canonicalize gives platform stubs readable named
//! identities; link cleanup retracts entries whose content is purely a description of a link that
//! exists. Each pass commits under [`Authority::Agent`](crate::memory::memory_block::Authority::Agent),
//! which permits cross-teller supersede and free `same_as` assertion while blocking `self` writes.
//!
//! The passes are cursor-resumed and idempotent, so an idle tick is cheap. A pass that finds nothing
//! to do still advances its cursor, so it does not re-scan the same window.

pub mod canonicalize;
pub mod consolidation;
pub mod link_cleanup;

mod scheduling;

pub use scheduling::activity_gate;
