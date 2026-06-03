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

use crate::{
    clock::Clock,
    event::{Cardinality, EventPayload, EventSource, PromptTemplateName, Teller, Visibility},
    ids::{EntryId, MemoryId, MemoryName, RelationName, Seq, TagName},
    settings::Settings,
    store::{Store, StoreError},
};

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
        tracing::debug!("genesis already complete; nothing to roll out");
        return Ok(Rollout::AlreadyComplete);
    }

    let mut templates_present: BTreeSet<(PromptTemplateName, u32)> = BTreeSet::new();
    let mut relations_present: BTreeSet<String> = BTreeSet::new();
    let mut tags_present: BTreeSet<String> = BTreeSet::new();
    let mut config_present = false;
    let mut self_present = false;
    for event in &existing {
        match &event.payload {
            EventPayload::PromptTemplateRegistered { name, version, .. } => {
                templates_present.insert((*name, *version));
            }
            EventPayload::LinkTypeRegistered { name, .. } => {
                relations_present.insert(name.as_str().to_owned());
            }
            EventPayload::TagCreated { name, .. } => {
                tags_present.insert(name.as_str().to_owned());
            }
            EventPayload::ConfigSet { .. } => {
                config_present = true;
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
        if !templates_present.contains(&(template.name, template.version)) {
            to_emit.push(EventPayload::PromptTemplateRegistered {
                name: template.name,
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

    for tag in seed_tags() {
        if !tags_present.contains(tag.name) {
            to_emit.push(EventPayload::TagCreated {
                name: TagName::new(tag.name),
                description: tag.description.to_owned(),
            });
        }
    }

    if !config_present {
        to_emit.push(EventPayload::ConfigSet {
            settings: Settings::default(),
            source: EventSource::Bootstrap,
        });
    }

    if !self_present {
        let self_id = MemoryId::generate();
        to_emit.push(EventPayload::MemoryCreated {
            id: self_id,
            name: MemoryName::new("self"),
        });
        // The persona is the agent's charter: a seed content entry, not a description. Entries are
        // immutable and append-only, so the authored voice never drifts, while the self can still
        // evolve as the agent appends further self-observations. The system prompt draws the
        // agent's identity from these entries verbatim, never from the regenerable description.
        for text in std::iter::once(&seed.persona).chain(&seed.seed_entries) {
            to_emit.push(EventPayload::MemoryContentAppended {
                id: self_id,
                entry_id: EntryId::generate(),
                asserted_at: clock.now(),
                text: text.clone(),
                told_by: Teller::Bootstrap,
                told_in: None,
                visibility: Visibility::Public,
            });
        }
    }

    let template_versions: BTreeMap<String, u32> = templates
        .iter()
        .map(|t| (t.name.as_str().to_owned(), t.version))
        .collect();
    to_emit.push(EventPayload::GenesisCompleted {
        manifest_hash: manifest_hash(seed, &templates),
        template_versions,
    });

    let events_emitted = to_emit.len();
    store.append(clock.now(), to_emit)?;
    tracing::info!(events_emitted, agent = %seed.agent_name, "rolled out genesis");
    Ok(Rollout::Created { events_emitted })
}

fn is_genesis_completed(event: &crate::event::Event) -> bool {
    matches!(event.payload, EventPayload::GenesisCompleted { .. })
}

/// A build-default prompt template. Bodies are first-pass placeholders; final wording is authored
/// by the build over time (spec §Initialization: prompt content is deferred to the build).
struct TemplateDef {
    name: PromptTemplateName,
    version: u32,
    body: &'static str,
}

fn default_templates() -> Vec<TemplateDef> {
    vec![
        TemplateDef {
            name: PromptTemplateName::Scaffold,
            version: 1,
            body: "You act through a persistent memory that you read and write by emitting Lua \
                   through the run_lua tool. A turn is a loop of steps: at each step you either \
                   call run_lua or give a reply. What you write to memory persists across sessions; \
                   your in-block scratchpad does not. You speak with several participants, who do \
                   not all see the same things.\n\n\
                   Memories are namespaced by kind: person/ for people, place/ for places, topic/ \
                   for subjects, context/ for conversations, and self for you. Read a merged \
                   identity through its canonical handle (person/phil, not a per-platform stub) so \
                   you do not look in the wrong place and miss what you know.\n\n\
                   Record your own observations and inferences under the `agent` teller. Keep \
                   confidences compartmentalized: content told to you in confidence is marked, and \
                   you do not repeat it to participants who should not see it.",
        },
        TemplateDef {
            name: PromptTemplateName::DescriptionRegen,
            version: 1,
            body: "You synthesize a memory's description from its content entries. Given the \
                   memory's name and entries, write a concise third-person description of who or \
                   what it is and the durable facts that matter. Call the `describe` tool with that \
                   description as plain prose — no preamble, headings, notes, or first-person \
                   framing — synthesizing only from the entries given.",
        },
        TemplateDef {
            name: PromptTemplateName::TemporalExtraction,
            version: 1,
            body: "<draft temporal-extraction template>",
        },
        TemplateDef {
            name: PromptTemplateName::Flush,
            version: 1,
            body: "This conversation session is ending and its live transcript is about to scroll \
                   out of view. Before it does, write to memory — by emitting Lua through the \
                   run_lua tool — anything from it worth keeping that you have not already recorded: \
                   facts you learned, decisions made, and commitments given. Record your own \
                   observations and inferences under the `agent` teller. Keep confidences \
                   compartmentalized exactly as in an ordinary turn. For threads still open, link \
                   the relevant memories `active_in` the current context, and clear `active_in` on \
                   threads that have closed, so the next session resurfaces what is still live. \
                   Nothing you leave only in the transcript survives, so be deliberate; when you \
                   have flushed what matters, reply briefly to confirm.",
        },
    ]
}

/// A build-seeded system tag. Like the seed relations, these are build defaults rather than part of
/// the genesis manifest hash, so adding one does not perturb an existing agent's hash.
struct TagDef {
    name: &'static str,
    description: &'static str,
}

fn seed_tags() -> Vec<TagDef> {
    vec![TagDef {
        name: "confidential",
        description: "Marks a context as confidential: asides told in a room carrying this tag are \
                      surfaced elsewhere flagged as confidential, and the tag is visible regardless \
                      of who is present.",
    }]
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
        hasher.update(template.name.as_str().as_bytes());
        hasher.update(template.version.to_le_bytes());
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
