//! Genesis and boot at the log level. An agent is created by rolling out the first events of its
//! log; boot branches on whether that rollout completed (spec §Initialization).
//!
//! Genesis is **idempotent**: re-driving it emits only the events that are missing, keyed on
//! content-stable identities (templates on `(name, version)`, relations and config on their key,
//! `self` on its unique name) rather than on freshly-minted ULIDs. So an interrupted creation
//! resumes cleanly, and `GenesisCompleted`'s `manifest_hash` — computed over content, not minted
//! ids — is stable across resumes. Boot keys off the presence of `GenesisCompleted`, never log
//! emptiness, so a crash mid-genesis is never mistaken for a born agent.

use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

use crate::clock::Clock;
use crate::event::{Cardinality, ConfigValue, EventPayload, EventSource};
use crate::ids::{EntryId, MemoryId, MemoryName, RelationName, Seq};
use crate::store::{Store, StoreError};

/// The seed identity an operator provides at creation: a name for the agent, a one-line persona,
/// and optional seed disposition entries. A freshly-born agent knows nothing else — genesis seeds
/// no `created_by` link and no facts about anyone (spec §Initialization).
#[derive(Clone, Debug)]
pub struct SeedSelf {
    pub agent_name: String,
    pub persona: String,
    pub seed_entries: Vec<String>,
}

/// What boot finds in the log. Boot branches on this, not on emptiness.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GenesisStatus {
    /// No events — direct the operator to create the agent.
    Empty,
    /// Events present but no `GenesisCompleted` — an interrupted genesis to re-drive.
    Incomplete,
    /// `GenesisCompleted` present — a born agent, ready to serve.
    Complete,
}

/// The outcome of a rollout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rollout {
    /// Genesis was already complete; nothing was emitted.
    AlreadyComplete,
    /// Genesis ran, emitting this many events (the full sequence on a fresh log, or just the
    /// missing tail when resuming an interrupted one).
    Created { events_emitted: usize },
}

/// Classify the log for boot.
pub fn status(store: &dyn Store) -> Result<GenesisStatus, StoreError> {
    let events = store.read_from(Seq::ZERO)?;
    if events.is_empty() {
        Ok(GenesisStatus::Empty)
    } else if events.iter().any(is_genesis_completed) {
        Ok(GenesisStatus::Complete)
    } else {
        Ok(GenesisStatus::Incomplete)
    }
}

/// Roll out genesis idempotently: emit the build's default templates, seed relations, default
/// config, and the seed `self`, skipping anything already present, then `GenesisCompleted`. The
/// whole tail commits as one atomic append.
pub fn rollout(
    store: &mut dyn Store,
    clock: &dyn Clock,
    seed: &SeedSelf,
) -> Result<Rollout, StoreError> {
    let existing = store.read_from(Seq::ZERO)?;
    if existing.iter().any(is_genesis_completed) {
        return Ok(Rollout::AlreadyComplete);
    }

    let mut templates_present: BTreeSet<(String, u32)> = BTreeSet::new();
    let mut relations_present: BTreeSet<String> = BTreeSet::new();
    let mut configs_present: BTreeSet<String> = BTreeSet::new();
    let mut self_present = false;
    for event in &existing {
        match &event.payload {
            EventPayload::PromptTemplateRegistered { name, version, .. } => {
                templates_present.insert((name.clone(), *version));
            }
            EventPayload::LinkTypeRegistered { name, .. } => {
                relations_present.insert(name.as_str().to_owned());
            }
            EventPayload::ConfigSet { key, .. } => {
                configs_present.insert(key.clone());
            }
            EventPayload::MemoryCreated { name, .. } if name.as_str() == "self" => {
                self_present = true;
            }
            _ => {}
        }
    }

    let templates = default_templates();
    let mut to_emit: Vec<EventPayload> = Vec::new();

    for template in &templates {
        if !templates_present.contains(&(template.name.to_owned(), template.version)) {
            to_emit.push(EventPayload::PromptTemplateRegistered {
                name: template.name.to_owned(),
                version: template.version,
                body: template.body.to_owned(),
                source: EventSource::Orchestration,
            });
        }
    }

    for relation in seed_relations() {
        if !relations_present.contains(relation.name) {
            to_emit.push(EventPayload::LinkTypeRegistered {
                name: RelationName::new(relation.name),
                inverse: RelationName::new(relation.inverse),
                from_card: relation.from_card,
                to_card: relation.to_card,
                symmetric: relation.symmetric,
                reflexive: relation.reflexive,
            });
        }
    }

    for (key, value) in default_config() {
        if !configs_present.contains(&key) {
            to_emit.push(EventPayload::ConfigSet {
                key,
                value,
                source: EventSource::Bootstrap,
            });
        }
    }

    if !self_present {
        let self_id = MemoryId::generate();
        to_emit.push(EventPayload::MemoryCreated {
            id: self_id,
            name: MemoryName::new("self"),
        });
        for text in &seed.seed_entries {
            to_emit.push(EventPayload::MemoryContentAppended {
                id: self_id,
                entry_id: EntryId::generate(),
                asserted_at: clock.now(),
                text: text.clone(),
            });
        }
    }

    let template_versions: BTreeMap<String, u32> = templates
        .iter()
        .map(|t| (t.name.to_owned(), t.version))
        .collect();
    to_emit.push(EventPayload::GenesisCompleted {
        manifest_hash: manifest_hash(seed, &templates),
        template_versions,
    });

    let events_emitted = to_emit.len();
    store.append(clock.now(), to_emit)?;
    Ok(Rollout::Created { events_emitted })
}

fn is_genesis_completed(event: &crate::event::Event) -> bool {
    matches!(event.payload, EventPayload::GenesisCompleted { .. })
}

/// A build-default prompt template. Bodies are first-pass placeholders; final wording is authored
/// by the build over time (spec §Initialization: prompt content is deferred to the build).
struct TemplateDef {
    name: &'static str,
    version: u32,
    body: &'static str,
}

fn default_templates() -> Vec<TemplateDef> {
    vec![
        TemplateDef {
            name: "scaffold",
            version: 1,
            body: "<draft system-prompt scaffold — see docs/spec.md §System prompt>",
        },
        TemplateDef {
            name: "description-regen",
            version: 1,
            body: "<draft description-regeneration template>",
        },
        TemplateDef {
            name: "temporal-extraction",
            version: 1,
            body: "<draft temporal-extraction template>",
        },
    ]
}

struct RelationDef {
    name: &'static str,
    inverse: &'static str,
    from_card: Cardinality,
    to_card: Cardinality,
    symmetric: bool,
    reflexive: bool,
}

fn seed_relations() -> Vec<RelationDef> {
    use Cardinality::{Many, One};
    vec![
        // created_by is historical origin (one creator); distinct from current operatorship.
        RelationDef {
            name: "created_by",
            inverse: "created",
            from_card: One,
            to_card: Many,
            symmetric: false,
            reflexive: false,
        },
        RelationDef {
            name: "operator_of",
            inverse: "operates",
            from_card: Many,
            to_card: Many,
            symmetric: false,
            reflexive: false,
        },
        RelationDef {
            name: "knows",
            inverse: "known_by",
            from_card: Many,
            to_card: Many,
            symmetric: false,
            reflexive: false,
        },
        // Cross-platform identity: symmetric, and its own inverse.
        RelationDef {
            name: "same_as",
            inverse: "same_as",
            from_card: Many,
            to_card: Many,
            symmetric: true,
            reflexive: false,
        },
        // A memory flagged live in a context; used by compaction carryover.
        RelationDef {
            name: "active_in",
            inverse: "has_active",
            from_card: Many,
            to_card: Many,
            symmetric: false,
            reflexive: false,
        },
    ]
}

/// Default behavioral tunables, with the concrete values the spec sets so the system is buildable
/// and testable from day one (spec §Time → search scoring, §Initialization → configuration).
fn default_config() -> Vec<(String, ConfigValue)> {
    use ConfigValue::{Float, Int};
    [
        ("compaction_token_budget", Int(24_000)),
        ("idle_gap_seconds", Int(1_800)),
        ("carryover_char_budget", Int(4_000)),
        ("brief_token_budget", Int(2_000)),
        ("recent_facts_count", Int(8)),
        ("present_set_cap", Int(10)),
        ("max_steps", Int(12)),
        ("search_weight_cosine", Float(0.5)),
        ("search_weight_bm25", Float(0.3)),
        ("search_weight_tag", Float(0.2)),
        ("recency_bonus_max", Float(0.3)),
        ("recency_tau_high_days", Int(90)),
        ("recency_tau_medium_days", Int(365)),
        ("recency_tau_low_days", Int(3_650)),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value))
    .collect()
}

/// A content hash over the genesis manifest — the seed-self and the template versions — so it is
/// stable across resumes and independent of minted ids (spec §Initialization).
fn manifest_hash(seed: &SeedSelf, templates: &[TemplateDef]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(seed.agent_name.as_bytes());
    hasher.update([0]);
    hasher.update(seed.persona.as_bytes());
    hasher.update([0]);
    for entry in &seed.seed_entries {
        hasher.update(entry.as_bytes());
        hasher.update([0]);
    }
    for template in templates {
        hasher.update(template.name.as_bytes());
        hasher.update(template.version.to_le_bytes());
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
