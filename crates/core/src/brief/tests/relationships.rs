//! Relationship ranking and the participant budget: edges float by type weight, the cap keeps the
//! highest-ranked edges with recency breaking ties, the char budget and the speaker guarantee decide
//! which present participants keep a full block, and a pre-pairing brief reconstructs its endpoints.
use super::{
    appended, compose_at_epoch, compose_at_epoch_answering, created, linked, materialized,
    register_relation, relationship_lines,
};
use crate::{
    brief::{self, Brief, BriefRelationship},
    event::{Teller, Visibility},
    ids::{MemoryId, MemoryName},
    settings::Settings,
    time::Timestamp,
    vocabulary::RelationName,
};

#[test]
fn relationships_are_ranked_by_type_weight() {
    // A hub touches an acquaintance edge (`knows`, low weight) and a structural origin edge
    // (`created_by`, top weight). However the edges are laid down, the brief floats the structural
    // relation above the social one — so "who created you" survives even a tight cap.
    let hub = MemoryId::generate();
    let creator = MemoryId::generate();
    let friend = MemoryId::generate();
    let (_store, graph) = materialized(vec![
        register_relation("knows", "known_by"),
        register_relation("created_by", "created"),
        created(hub, "person/hub"),
        created(creator, "agent/self"),
        created(friend, "person/friend"),
        // The acquaintance edge is created first, so a stable-but-unranked list would show it first.
        linked(hub, friend, "knows"),
        linked(hub, creator, "created_by"),
    ]);

    let brief = brief::compose_participant_brief(
        &graph,
        hub,
        &[hub],
        &Settings::default().brief,
        Timestamp::from_millis(0),
    )
    .unwrap()
    .unwrap();
    let lines = relationship_lines(&brief.render());
    assert_eq!(
        lines,
        vec![
            "- person/hub → created_by → agent/self".to_owned(),
            "- person/hub → knows → person/friend".to_owned(),
        ],
        "the structural created_by edge ranks above the social knows edge"
    );
}

#[test]
fn the_relationships_cap_keeps_the_highest_ranked_edges() {
    // A hub floods its own block with many `knows` edges. With the cap at two, the structural
    // `created_by` is kept and, among the equal-weight `knows` edges, the neighbour with the most
    // recent visible activity wins the remaining slot — recency breaks the type-weight tie.
    let hub = MemoryId::generate();
    let creator = MemoryId::generate();
    let stale = MemoryId::generate();
    let fresh = MemoryId::generate();
    let idle = MemoryId::generate();
    let (_store, graph) = materialized(vec![
        register_relation("knows", "known_by"),
        register_relation("created_by", "created"),
        created(hub, "person/hub"),
        created(creator, "agent/self"),
        created(stale, "person/stale"),
        created(fresh, "person/fresh"),
        created(idle, "person/idle"),
        // Distinct recency: `fresh` is the most recently active acquaintance; `idle` has no activity.
        appended(
            stale,
            1_000,
            "older news",
            Teller::Agent,
            Visibility::Public,
        ),
        appended(
            fresh,
            5_000,
            "recent news",
            Teller::Agent,
            Visibility::Public,
        ),
        linked(hub, creator, "created_by"),
        linked(hub, stale, "knows"),
        linked(hub, fresh, "knows"),
        linked(hub, idle, "knows"),
    ]);

    let mut settings = Settings::default().brief;
    settings.key_relationships = 2;
    let brief =
        brief::compose_participant_brief(&graph, hub, &[hub], &settings, Timestamp::from_millis(0))
            .unwrap()
            .unwrap();
    let targets: Vec<&str> = brief
        .relationships
        .iter()
        .map(|relationship| relationship.target.as_str())
        .collect();
    assert_eq!(
        targets,
        vec!["agent/self", "person/fresh"],
        "created_by is kept by weight, and the most recently active knows-neighbour wins the tie"
    );
}

#[test]
fn the_char_budget_collapses_lower_ranked_participants_to_name_only() {
    // Two present participants each carry a full block. With the budget set to exactly the present
    // header plus the top-ranked participant's block, the recency winner keeps its full block and the
    // other collapses to a name-only line — the participant-axis flood is packed, not truncated.
    let fresh = MemoryId::generate();
    let stale = MemoryId::generate();
    let (_store, graph) = materialized(vec![
        created(fresh, "person/fresh"),
        created(stale, "person/stale"),
        appended(
            fresh,
            5_000,
            "fresh has news to share",
            Teller::Agent,
            Visibility::Public,
        ),
        appended(
            stale,
            1_000,
            "stale has quiet news",
            Teller::Agent,
            Visibility::Public,
        ),
    ]);
    let present_set = [fresh, stale];

    // Measure the top-ranked block against a generous budget, then set the budget to admit only it.
    let mut generous = Settings::default().brief;
    generous.char_budget = i64::MAX;
    let fresh_block = brief::compose_participant(
        &graph,
        fresh,
        &present_set,
        &generous,
        Timestamp::from_millis(0),
    )
    .unwrap();
    let budget = "# Present\n".chars().count() + fresh_block.chars().count();

    let mut settings = Settings::default().brief;
    settings.char_budget = budget as i64;
    let out = compose_at_epoch(&graph, &settings, &present_set, None, &[]);

    assert!(out.contains("fresh has news to share")); // the recency winner keeps its full block
    assert!(out.contains("- person/stale (present)")); // the other collapses to name-only
    assert!(!out.contains("stale has quiet news")); // ...and its facts do not surface
}

#[test]
fn the_active_speaker_is_guaranteed_a_full_block_over_the_recency_winner() {
    // The shape of issue #85: two participants present, the recency winner is *not* the one speaking.
    // With the budget set to admit only one full block, the recency winner would ordinarily take it and
    // the speaker collapse to name-only — but the speaker is guaranteed, so it keeps its full block and
    // the recency winner is the one that collapses.
    let speaker = MemoryId::generate();
    let winner = MemoryId::generate();
    let (_store, graph) = materialized(vec![
        created(speaker, "person/rowan"),
        created(winner, "person/wren"),
        appended(
            speaker,
            1_000,
            "rowan is the one asking",
            Teller::Agent,
            Visibility::Public,
        ),
        appended(
            winner,
            5_000,
            "wren was touched more recently",
            Teller::Agent,
            Visibility::Public,
        ),
    ]);
    let present_set = [speaker, winner];

    // Budget to admit exactly the present header plus the speaker's own full block. Without the
    // guarantee the recency winner (wren) would win this single slot.
    let mut generous = Settings::default().brief;
    generous.char_budget = i64::MAX;
    let speaker_block = brief::compose_participant(
        &graph,
        speaker,
        &present_set,
        &generous,
        Timestamp::from_millis(0),
    )
    .unwrap();
    let budget = "# Present\n".chars().count() + speaker_block.chars().count();

    let mut settings = Settings::default().brief;
    settings.char_budget = budget as i64;
    let out = compose_at_epoch_answering(&graph, &settings, &present_set, &[speaker], None, &[]);

    assert!(out.contains("rowan is the one asking")); // the speaker keeps its full block...
    assert!(out.contains("## person/rowan"));
    assert!(out.contains("- person/wren (present)")); // ...and the recency winner collapses instead
    assert!(!out.contains("wren was touched more recently"));
    // The speaker renders ahead of the remaining present participants.
    assert!(out.find("## person/rowan").unwrap() < out.find("person/wren").unwrap());
}

#[test]
fn the_guaranteed_speaker_set_stays_bounded_by_the_cap() {
    // The speaker guarantee is priority within the cap, not an unbounded escape from it: with three
    // speakers present under a cap of two, only the two most-recent speakers get a full block and the
    // third collapses to a name-only line, so a large inbound batch of distinct senders cannot balloon
    // the brief past the cap.
    let older = MemoryId::generate();
    let middle = MemoryId::generate();
    let newest = MemoryId::generate();
    let (_store, graph) = materialized(vec![
        created(older, "person/quinn"),
        created(middle, "person/rowan"),
        created(newest, "person/wren"),
        appended(
            older,
            1_000,
            "quinn spoke",
            Teller::Agent,
            Visibility::Public,
        ),
        appended(
            middle,
            2_000,
            "rowan spoke",
            Teller::Agent,
            Visibility::Public,
        ),
        appended(
            newest,
            3_000,
            "wren spoke",
            Teller::Agent,
            Visibility::Public,
        ),
    ]);
    let present_set = [older, middle, newest];

    // A generous budget so only the cap — not the budget — bounds the full blocks.
    let mut settings = Settings::default().brief;
    settings.char_budget = i64::MAX;
    settings.present_set_cap = 2;
    let out = compose_at_epoch_answering(&graph, &settings, &present_set, &present_set, None, &[]);

    // The two most-recent speakers keep full blocks; the oldest speaker is the one over the cap.
    assert!(out.contains("## person/wren"));
    assert!(out.contains("## person/rowan"));
    assert!(out.contains("- person/quinn (present)"));
    assert!(!out.contains("## person/quinn"));
    assert_eq!(out.matches("## person/").count(), 2); // exactly the cap, never more
}

#[test]
fn a_pre_pairing_brief_reconstructs_relationship_endpoints() {
    // A brief recorded before a relationship named both endpoints stored only the neighbour as
    // `subject`, with the near end implicit and the edge rendered outgoing. Deserializing such a log
    // must reconstruct `source` (this brief's own subject) and `target` (the neighbour), so the
    // historical join renders as it did — subject → neighbour — rather than failing to load.
    let json = r#"{
        "subject":"person/priya",
        "summary":null,
        "recent_facts":[],
        "relationships":[{"relation":"knows","subject":"person/erin","marker":null}]
    }"#;
    let brief: Brief = serde_json::from_str(json).unwrap();
    assert_eq!(
        brief.relationships,
        vec![BriefRelationship {
            relation: RelationName::new("knows"),
            source: MemoryName::new("person/priya"),
            target: MemoryName::new("person/erin"),
            marker: None,
        }]
    );
    // And the current form (endpoints named) round-trips through the same wire.
    let current = serde_json::to_string(&brief).unwrap();
    assert_eq!(serde_json::from_str::<Brief>(&current).unwrap(), brief);
}
