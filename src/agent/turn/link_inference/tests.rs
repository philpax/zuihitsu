use crate::{
    agent::turn::link_inference::{
        link_inference_argument, prompt::render_prompt, relations::ExistingLink,
    },
    event::{Cardinality, Teller, Visibility},
    graph::{EntryOrigin, EntryView, MemoryView, RelationView},
    ids::{EntryId, MemoryId, MemoryName},
    time::Timestamp,
    vocabulary::RelationName,
};

fn memory(name: &str) -> MemoryView {
    MemoryView {
        id: MemoryId::generate(),
        name: MemoryName::new(name),
        description: "a memory".to_owned(),
        volatility: crate::event::Volatility::Medium,
        created_at: Timestamp::from_millis(0),
        tags: Vec::new(),
    }
}

fn entry(text: &str) -> EntryView {
    EntryView {
        entry_id: EntryId::generate(),
        asserted_at: Timestamp::from_millis(1_000),
        occurred_sort: None,
        occurred_at: None,
        occurred_authored: false,
        text: text.to_owned(),
        told_by: Teller::Agent,
        told_in: None,
        visibility: Visibility::Public,
        superseded_by: None,
        retracted_reason: None,
        origin: EntryOrigin::Recorded,
        attestations: Vec::new(),
    }
}

fn link(from: &str, relation: &str, to: &str) -> ExistingLink {
    ExistingLink {
        from_id: MemoryId::generate(),
        to_id: MemoryId::generate(),
        from: MemoryName::new(from),
        to: MemoryName::new(to),
        relation: RelationName::new(relation),
    }
}

fn relation(name: &str, inverse: &str, description: &str) -> RelationView {
    RelationView {
        name: RelationName::new(name),
        inverse: RelationName::new(inverse),
        from_card: Cardinality::One,
        to_card: Cardinality::Many,
        symmetric: false,
        reflexive: false,
        description: description.to_owned(),
    }
}

#[test]
fn a_well_formed_reply_parses_into_relations_and_links() {
    let reply = serde_json::json!({
        "new_relations": [{
            "name": "authored_by",
            "inverse": "authored",
            "from_card": "many",
            "to_card": "one",
            "symmetric": false,
            "reflexive": false
        }],
        "links": [{
            "entry": 1,
            "subject": "topic/novel",
            "relation": "authored_by",
            "object": "person/clara"
        }]
    });
    let args = link_inference_argument(&reply).expect("a well-formed reply parses");
    assert_eq!(args.new_relations.len(), 1);
    assert_eq!(args.new_relations[0].name, "authored_by");
    assert_eq!(args.new_relations[0].inverse, "authored");
    assert_eq!(args.links.len(), 1);
    assert_eq!(args.links[0].subject, "topic/novel");
    assert_eq!(args.links[0].object, "person/clara");
}

#[test]
fn a_malformed_new_relation_is_skipped_while_links_survive() {
    let reply = serde_json::json!({
        "new_relations": [{ "name": "authored_by" }],
        "links": [{
            "entry": 1,
            "subject": "person/dave",
            "relation": "knows",
            "object": "person/clara"
        }]
    });
    let args = link_inference_argument(&reply).expect("the links are salvaged");
    assert!(args.new_relations.is_empty());
    assert_eq!(args.links.len(), 1);
    assert_eq!(args.links[0].relation, "knows");
}

#[test]
fn a_malformed_link_is_skipped_while_relations_survive() {
    let reply = serde_json::json!({
        "new_relations": [{
            "name": "authored_by",
            "inverse": "authored",
            "from_card": "many",
            "to_card": "one",
            "symmetric": false,
            "reflexive": false
        }],
        "links": [{ "entry": 1, "relation": "authored_by" }]
    });
    let args = link_inference_argument(&reply).expect("the relations are salvaged");
    assert_eq!(args.new_relations.len(), 1);
    assert!(args.links.is_empty());
}

#[test]
fn a_reply_with_no_links_or_relations_parses_to_empty() {
    let reply = serde_json::json!({ "new_relations": [], "links": [] });
    let args = link_inference_argument(&reply).expect("an empty reply parses");
    assert!(args.new_relations.is_empty());
    assert!(args.links.is_empty());
}

/// A registered relation renders with its values substituted, not as literal placeholders. The full
/// frame is not restated here — only its `{{name}}`/`{{inverse}}` markers must never reach the
/// model as text.
#[test]
fn a_relation_renders_substituted_not_as_literal_placeholders() {
    let prompt = render_prompt(
        &memory("person/dave"),
        &[entry("Dave mentors Clara.")],
        &[link("person/dave", "mentors", "person/clara")],
        &[relation(
            "mentors",
            "mentees",
            "the mentor teaches the mentee",
        )],
        &[memory("person/clara")],
        Timestamp::from_millis(1_725_000_000_000),
    );
    assert!(
        prompt.contains("- mentors/mentees — a link \"A mentors B\" restates as \"B mentees A\""),
        "the relation renders with substituted values: {prompt}"
    );
    assert!(
        !prompt.contains("{name}") && !prompt.contains("{inverse}") && !prompt.contains("{{name}}"),
        "no placeholder text reaches the model: {prompt}"
    );
}
