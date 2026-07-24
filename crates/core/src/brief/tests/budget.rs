//! Char-budget behaviour and the mandatory self block: an unaffordable active thread drops with its
//! header rather than truncating mid-body, the `# You` block renders its authored entries without a
//! summary, and a zero budget still renders self in full while the optional participant blocks
//! collapse to name-only lines.
use crate::{
    brief::tests::{appended, compose_at_epoch, created, materialized},
    event::{EventPayload, EventSource, Teller, Visibility},
    graph::Graph,
    ids::MemoryId,
    settings::Settings,
    store::{MemoryStore, Store},
    time::Timestamp,
};

#[test]
fn an_active_thread_the_budget_cannot_afford_drops_with_its_header() {
    // The active-threads section — cold-open-derived here, an absent memory in the working set — is
    // packed per thread under the char budget, and its header is charged only if a thread is admitted.
    // With the budget spent on the mandatory blocks, the thread and its header both drop rather than
    // truncating the thread mid-body.
    let absent = MemoryId::generate();
    let (_store, graph) = materialized(vec![
        created(absent, "person/absent"),
        appended(
            absent,
            1_000,
            "the absent thread's fact",
            Teller::Agent,
            Visibility::Public,
        ),
    ]);

    // A generous budget admits the thread: the section and its fact both render.
    let mut generous = Settings::default().brief;
    generous.char_budget = i64::MAX;
    let full = compose_at_epoch(&graph, &generous, &[], None, &[absent]);
    assert!(full.contains("# Active threads"));
    assert!(full.contains("the absent thread's fact"));

    // A zero budget cannot afford it: neither the header nor the body surfaces.
    let mut tight = Settings::default().brief;
    tight.char_budget = 0;
    let out = compose_at_epoch(&graph, &tight, &[], None, &[absent]);
    assert!(!out.contains("# Active threads"));
    assert!(!out.contains("the absent thread's fact"));
}

#[test]
fn the_self_block_renders_entries_without_a_summary() {
    // The `# You` block drops the generated summary and renders its authored entries only: on the self
    // memory the entries are canonical, and the summary would only restate them at the cost of budget
    // the present set needs (issue #85).
    let agent = MemoryId::generate();
    let mut store = MemoryStore::new();
    let mut graph = Graph::open_in_memory().unwrap();
    for payload in [
        created(agent, "self"),
        EventPayload::MemoryDescriptionRegenerated {
            id: agent,
            new_text: "a generated summary of who the agent is".to_owned(),
            produced_by: None,
        },
        appended(
            agent,
            1_000,
            "the agent's own authored charter",
            Teller::Agent,
            Visibility::Public,
        ),
    ] {
        let committed = store
            .append(Timestamp::from_millis(0), EventSource::Agent, vec![payload])
            .unwrap();
        for event in committed {
            graph.apply(&event).unwrap();
        }
    }

    let settings = Settings::default().brief;
    let out = compose_at_epoch(&graph, &settings, &[], None, &[]);

    assert!(out.contains("# You"));
    assert!(out.contains("the agent's own authored charter")); // the entry surfaces...
    assert!(!out.contains("<summary>")); // ...but no summary block is rendered for self
    assert!(!out.contains("a generated summary of who the agent is"));
}

#[test]
fn a_zero_char_budget_still_renders_the_mandatory_self_block() {
    // With the budget at zero, the self block — who the agent is — must still render in full; only the
    // optional participant blocks collapse. A budget can bound the brief, never erase the agent's own
    // memory.
    let agent = MemoryId::generate();
    let other = MemoryId::generate();
    let (_store, graph) = materialized(vec![
        created(agent, "self"),
        created(other, "person/other"),
        appended(
            agent,
            1_000,
            "the agent's own charter fact",
            Teller::Agent,
            Visibility::Public,
        ),
        appended(
            other,
            1_000,
            "the other's fact",
            Teller::Agent,
            Visibility::Public,
        ),
    ]);

    let mut settings = Settings::default().brief;
    settings.char_budget = 0;
    let out = compose_at_epoch(&graph, &settings, &[other], None, &[]);

    assert!(out.contains("# You"));
    assert!(out.contains("the agent's own charter fact")); // self is mandatory, renders in full
    assert!(out.contains("- person/other (present)")); // the participant collapses to name-only
    assert!(!out.contains("the other's fact")); // ...and its facts do not surface
}
