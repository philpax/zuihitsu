//! A fresh log rolls out a complete agent, an interrupted one resumes by emitting only what's
//! missing, and a complete one is left alone — all keyed on the presence of `GenesisCompleted`,
//! never log emptiness (spec §Initialization).

mod relations;
mod rollout;
mod scaffold;

use crate::{
    InstanceFeatures,
    agent::genesis::{self, GenesisStatus, Rollout, SeedSelf},
    clock::ManualClock,
    event::PromptTemplateName,
    ids::Seq,
    settings::Settings,
    store::{MemoryStore, Store},
    time::Timestamp,
};

pub(super) fn seed() -> SeedSelf {
    SeedSelf {
        agent_name: "Kestrel".to_owned(),
        persona: "A thoughtful, discreet companion with a long memory.".to_owned(),
        seed_entries: vec!["I keep what people tell me in confidence.".to_owned()],
    }
}

pub(super) fn clock() -> ManualClock {
    ManualClock::new(Timestamp::from_millis(1_000))
}

pub(super) fn genesis_hash(store: &MemoryStore) -> String {
    store
        .read_from(crate::ids::Seq::ZERO)
        .unwrap()
        .into_iter()
        .find_map(|e| match e.payload {
            crate::event::EventPayload::GenesisCompleted { manifest_hash, .. } => {
                Some(manifest_hash)
            }
            _ => None,
        })
        .expect("genesis completed")
}

/// The Scaffold template body `default_templates` bakes for a feature set — the third gate the
/// transcripts feature must move in lockstep with (Lua registration and the API reference are the
/// other two).
pub(super) fn scaffold_body(features: &InstanceFeatures) -> String {
    super::default_templates(features)
        .into_iter()
        .find(|template| template.name == PromptTemplateName::Scaffold)
        .map(|template| template.body)
        .expect("default_templates includes the scaffold")
}
