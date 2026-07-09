//! Structured relationships: recording a typed link between people (`Knows`), and — the read side —
//! retrieving them back out of the graph with the link readers (`RecallsConnections`,
//! `DistinguishesMentorDirection`, `AttributesRelationshipToTeller`). `Knows` is a gating write oracle;
//! the read scenarios are metrics judged by whether the reply reflects the stored relationships.
//! `DistinguishesMentorDirection` is the one the readers are uniquely needed for: it puts the subject on
//! *both* sides of an asymmetric relation, so only reading the edge's direction (not a semantic search
//! that conflates the two) answers it — and it exercises the full write side, since mentorship is *not*
//! a seeded relation: the agent must register a directional mentorship relation itself and link both
//! directions the right way round before the read can come out right. `AttributesRelationshipToTeller`
//! checks the link's `told_by` provenance is legible: the agent must attribute a recorded relationship
//! to who asserted it, not to whoever is currently asking.
//!
//! Mentorship being learned rather than seeded (spec §Initialization: the seed set is a minimum-viable
//! ontology of structural universals, social semantics being the agent's to coin) is what gives
//! `DistinguishesMentorDirection` and `InfersLinkFromContent` their teeth: each accepts whatever label
//! the run mints from a small mentorship family (`mentor_of`/`mentors`/`mentored`/`mentored_by`), so it
//! tests that the agent reaches for a typed directional relation at all, not that it lands on a
//! build-blessed spelling.

mod attributes_relationship_to_teller;
mod distinguishes_mentor_direction;
mod infers_link_from_content;
mod knows;
mod recalls_connections;

use std::sync::Arc;

use async_trait::async_trait;
use zuihitsu::{
    ConversationId, ConversationLocator, EntryId, Event, EventPayload, Initiation, MemoryId,
    MemoryName, SessionId, Teller, Timestamp, TurnId, TurnRole, Visibility,
};

use crate::{
    analysis,
    context::RUN_START_MS,
    judge::Judge,
    package::{Bar, Category, ScenarioMeta, Verdict, VerdictKind},
    scenario::Scenario,
    step::{EvalStep, Turn},
};

use crate::scenarios::relations::{
    attributes_relationship_to_teller::AttributesRelationshipToTeller,
    distinguishes_mentor_direction::DistinguishesMentorDirection,
    infers_link_from_content::InfersLinkFromContent, knows::Knows,
    recalls_connections::RecallsConnections,
};

/// This module's scenarios.
pub fn scenarios() -> Vec<Arc<dyn Scenario>> {
    vec![
        Arc::new(Knows),
        Arc::new(RecallsConnections),
        Arc::new(DistinguishesMentorDirection),
        Arc::new(AttributesRelationshipToTeller),
        Arc::new(InfersLinkFromContent),
    ]
}
