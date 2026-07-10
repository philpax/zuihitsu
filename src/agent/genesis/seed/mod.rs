//! The genesis seed data: the default prompt-template scaffold, the seed tags and relations,
//! and the content-stable manifest hash computed over them.

mod templates;

use sha2::{Digest, Sha256};

use crate::{event::Cardinality, vocabulary::RelationName};

use super::{RelationDef, SeedSelf, TagDef, TemplateDef};

pub(super) use templates::default_templates;

pub(super) fn seed_tags() -> Vec<TagDef> {
    vec![TagDef {
        name: "confidential",
        description: "Marks a context as confidential: asides told in a room carrying this tag are \
                      surfaced elsewhere flagged as confidential, and the tag is visible regardless \
                      of who is present.",
    }]
}

/// The seed relations are a minimum-viable ontology: the structural universals the system itself
/// leans on — identity (`same_as`), participation (`participates_in`/`has_participant`), composition
/// (`part_of`/`contains`), origin (`created_by`/`created`), operatorship (`operator_of`/`operates`),
/// and acquaintance (`knows`/`known_by`). These earn seeding because they are domain-independent
/// scaffolding that any instance's graph is built out of, and because code matches on several of them
/// (`same_as` drives identity-class merging, and the rest anchor the reference examples and the
/// scaffold's placement teaching).
///
/// Social and environmental semantics — mentorship, venues, employment, and the rest — are
/// deliberately *not* seeded. They belong to the agent's own operating environment, so the agent
/// coins them itself (`links.register`) with names and directions that fit what it actually
/// encounters. Per-instance registrations persist in the log, so one agent's coined vocabulary is
/// stable across its whole life; which label a given instance mints (e.g. `mentors` versus
/// `mentored_by`) may vary between instances, and that is fine — what matters is that the agent
/// reaches for a typed relation at all, not that it lands on a build-blessed spelling.
pub(super) fn seed_relations() -> Vec<RelationDef> {
    use Cardinality::{Many, One};
    use RelationName::{
        Contains, Created, CreatedBy, HasParticipant, KnownBy, Knows, OperatedBy, OperatorOf,
        PartOf, ParticipatesIn, SameAs,
    };
    vec![
        RelationDef {
            name: CreatedBy,
            inverse: Created,
            from_card: One,
            to_card: Many,
            symmetric: false,
            reflexive: false,
            description: "A thing's historical origin — who created it. Distinct from current \
                operatorship.",
        },
        RelationDef {
            name: OperatorOf,
            inverse: OperatedBy,
            from_card: Many,
            to_card: Many,
            symmetric: false,
            reflexive: false,
            description: "Who currently operates or runs a thing — the present operator, not the \
                originator.",
        },
        RelationDef {
            name: Knows,
            inverse: KnownBy,
            from_card: Many,
            to_card: Many,
            symmetric: false,
            reflexive: false,
            description: "One person knows another — a person-to-person relationship.",
        },
        RelationDef {
            name: SameAs,
            inverse: SameAs,
            from_card: Many,
            to_card: Many,
            symmetric: true,
            reflexive: false,
            description: "Two platform stubs are the same person — cross-platform identity.",
        },
        RelationDef {
            name: ParticipatesIn,
            inverse: HasParticipant,
            from_card: Many,
            to_card: Many,
            symmetric: false,
            reflexive: false,
            description: "A person is in an event — someone who will be there or took part.",
        },
        RelationDef {
            name: PartOf,
            inverse: Contains,
            from_card: Many,
            to_card: Many,
            symmetric: false,
            reflexive: false,
            description: "An event, entry-bearing memory, or sub-topic belongs to a topic, \
                project, or workstream — membership or aboutness. Not for people, who \
                participates_in an event instead.",
        },
    ]
}

/// A content hash over the genesis manifest — the seed-self and the template versions — so it is
/// stable across resumes and independent of minted ids (spec §Initialization).
pub(super) fn manifest_hash(seed: &SeedSelf, templates: &[TemplateDef]) -> String {
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
