//! Composition against the present set: a confidential current-context tag, the subject-guard that
//! suppresses asides about a present subject, working-set re-filtering, the same-as class collapse,
//! the present-set cap versus the exclude predicate, and the contextual brief's lean rendering.
use std::collections::BTreeSet;

use crate::{
    brief::{
        self,
        tests::{appended, compose_at_epoch, created, linked, materialized, register_relation},
    },
    event::{Cardinality, EventPayload, LinkPosture, LinkSource, Teller, Visibility},
    ids::{EntryId, MemoryId},
    settings::Settings,
    time::Timestamp,
    vocabulary::{RelationName, TagName},
};

#[test]
fn current_room_brief_shows_confidential_regardless_of_present_set() {
    // Scenario 14: #leads is #confidential. A later session has Marcus and Dave but not the teller;
    // the current-context brief still shows confidential — it's a memory-level tag, not teller-gated.
    let leads = MemoryId::generate();
    let marcus = MemoryId::generate();
    let dave = MemoryId::generate();
    let (_store, graph) = materialized(vec![
        created(leads, "context/leads"),
        EventPayload::tag_created(TagName::new("confidential"), "confidential room"),
        EventPayload::tag_applied_to_memory(leads, TagName::new("confidential")),
        created(marcus, "person/marcus"),
        created(dave, "person/dave"),
    ]);

    let out = compose_at_epoch(
        &graph,
        &Settings::default().brief,
        &[marcus, dave],
        Some(leads),
        &[],
    );
    assert!(out.contains("Current room: #leads (confidential)"));
}

#[test]
fn a_corroborated_public_fact_carries_an_also_told_by_marker_in_the_brief() {
    // Marcus plays cello — a public fact the agent recorded, corroborated by erin's attributed
    // endorsement. The brief bakes an "also told by" marker naming the visible corroborator; a hidden
    // private confidence (frank, absent) leaves no residue.
    let marcus = MemoryId::generate();
    let erin = MemoryId::generate();
    let frank = MemoryId::generate();
    let entry = EntryId::generate();
    let (_store, graph) = materialized(vec![
        created(marcus, "person/marcus"),
        created(erin, "person/erin"),
        created(frank, "person/frank"),
        EventPayload::MemoryContentAppended {
            id: marcus,
            entry_id: entry,
            asserted_at: Timestamp::from_millis(1_000),
            occurred_at: None,
            text: "plays cello".to_owned(),
            told_by: Teller::Agent,
            told_in: None,
            visibility: Visibility::Public,
        },
        EventPayload::EntryAttested {
            memory: marcus,
            entry,
            teller: Teller::Participant(erin),
            told_in: None,
            asserted_at: Timestamp::from_millis(1_100),
            posture: Visibility::Attributed,
            phrasing: None,
            source_entry: None,
            produced_by: None,
        },
        EventPayload::EntryAttested {
            memory: marcus,
            entry,
            teller: Teller::Participant(frank),
            told_in: None,
            asserted_at: Timestamp::from_millis(1_200),
            posture: Visibility::PrivateToTeller,
            phrasing: Some("frank mentioned it in confidence".to_owned()),
            source_entry: None,
            produced_by: None,
        },
    ]);

    let out = compose_at_epoch(&graph, &Settings::default().brief, &[marcus], None, &[]);
    assert!(
        out.contains("plays cello [also told by person/erin]"),
        "{out}"
    );
    assert!(
        !out.contains("frank"),
        "the hidden attester leaves no residue: {out}"
    );
}

#[test]
fn an_aside_about_a_present_subject_is_suppressed_in_the_brief() {
    // Scenario 2 (composition half): Erin's private aside about Marcus. With Marcus present, his brief
    // block renders his public fact but the subject-guard suppresses the aside. (The surfaces-while-
    // absent half and the join injection complete at the join increment.)
    let marcus = MemoryId::generate();
    let erin = MemoryId::generate();
    let (_store, graph) = materialized(vec![
        created(marcus, "person/marcus"),
        created(erin, "person/erin"),
        appended(
            marcus,
            1_000,
            "on the platform team",
            Teller::Agent,
            Visibility::Public,
        ),
        appended(
            marcus,
            1_100,
            "is being managed out",
            Teller::Participant(erin),
            Visibility::PrivateToTeller,
        ),
    ]);

    let out = compose_at_epoch(
        &graph,
        &Settings::default().brief,
        &[erin, marcus],
        None,
        &[],
    );
    assert!(out.contains("on the platform team")); // Marcus's block renders
    assert!(!out.contains("is being managed out")); // ...but the aside is suppressed
}

#[test]
fn a_subject_joining_suppresses_asides_about_them() {
    // Scenario 2 (join half): Erin's private aside about Marcus. While only Erin is present it is
    // visible (it would surface to her). Marcus's join-brief is built against the now-present set
    // {Erin, Marcus}, where the subject-guard suppresses it — the dangerous direction is closed.
    let marcus = MemoryId::generate();
    let erin = MemoryId::generate();
    let (_store, graph) = materialized(vec![
        created(marcus, "person/marcus"),
        created(erin, "person/erin"),
        appended(
            marcus,
            1_000,
            "is being managed out",
            Teller::Participant(erin),
            Visibility::PrivateToTeller,
        ),
    ]);
    let settings = Settings::default().brief;

    // Before Marcus joins (only Erin present): the aside is visible.
    let before = brief::compose_participant(
        &graph,
        marcus,
        &[erin],
        &settings,
        Timestamp::from_millis(2_000),
    )
    .unwrap();
    assert!(before.contains("is being managed out"));

    // Marcus's join-brief, built against {Erin, Marcus}: the subject-guard suppresses it.
    let join_brief = brief::compose_participant(
        &graph,
        marcus,
        &[erin, marcus],
        &settings,
        Timestamp::from_millis(2_000),
    )
    .unwrap();
    assert!(!join_brief.contains("is being managed out"));
}

#[test]
fn the_working_set_is_re_filtered_against_the_new_present_set() {
    // The working set carried across a compaction is re-filtered through `visible` against the *new*
    // present set, never trusted from the old session: Erin's private aside about Marcus surfaces in
    // active threads while only Erin is present, but is suppressed once Marcus is present at the new
    // segment boundary (the safety property fixture 22 guards, at the deterministic level).
    let marcus = MemoryId::generate();
    let erin = MemoryId::generate();
    let (_store, graph) = materialized(vec![
        created(marcus, "person/marcus"),
        created(erin, "person/erin"),
        appended(
            marcus,
            1_000,
            "is being managed out",
            Teller::Participant(erin),
            Visibility::PrivateToTeller,
        ),
    ]);
    let settings = Settings::default().brief;

    // Marcus is in the working set. With only Erin present, the aside is visible in active threads.
    let only_erin = compose_at_epoch(&graph, &settings, &[erin], None, &[marcus]);
    assert!(only_erin.contains("# Active threads"));
    assert!(only_erin.contains("is being managed out"));

    // With Marcus present at the new boundary, the aside is suppressed — the working-set copy is
    // re-filtered against {Erin, Marcus} just like any other block.
    let with_marcus = compose_at_epoch(&graph, &settings, &[erin, marcus], None, &[marcus]);
    assert!(!with_marcus.contains("is being managed out"));
}

#[test]
fn a_same_as_class_collapses_to_a_single_block() {
    // One identity spanning a present stub and two working-set stubs (a person merged across
    // platforms). The class reads (`class_entries`, `class_neighbor_links`) already resolve the whole
    // class, so rendering each stub would repeat the same facts and relationships. The brief collapses
    // the class to one block, headed by the present stub, and drops the intra-class `same_as` plumbing.
    let direct = MemoryId::generate();
    let canonical = MemoryId::generate();
    let chat = MemoryId::generate();
    let erin = MemoryId::generate();
    let (_store, graph) = materialized(vec![
        EventPayload::LinkTypeRegistered {
            name: RelationName::SameAs,
            inverse: RelationName::SameAs,
            from_card: Cardinality::Many,
            to_card: Cardinality::Many,
            symmetric: true,
            reflexive: false,
            description: String::new(),
        },
        register_relation("knows", "known_by"),
        created(direct, "person/rowan@direct"),
        created(canonical, "person/rowan"),
        created(chat, "person/rowan@chat"),
        created(erin, "person/erin"),
        // Merge all three stubs into one class: direct ↔ canonical ↔ chat.
        EventPayload::link_created(
            direct,
            canonical,
            RelationName::SameAs,
            LinkPosture {
                source: LinkSource::Operator,
                told_by: None,
                told_in: None,
                visibility: Visibility::Public,
            },
        ),
        EventPayload::link_created(
            chat,
            canonical,
            RelationName::SameAs,
            LinkPosture {
                source: LinkSource::Operator,
                told_by: None,
                told_in: None,
                visibility: Visibility::Public,
            },
        ),
        // A distinctive class-wide fact, filed on a non-present stub — it merges across the class, so
        // before the collapse it rendered once per stub (three times).
        appended(
            canonical,
            1_000,
            "maintains an open-source build tool",
            Teller::Agent,
            Visibility::Public,
        ),
        // A summary on the present stub, to confirm it heads the single block.
        EventPayload::MemoryDescriptionRegenerated {
            id: direct,
            new_text: "An engineer known across two chat platforms.".to_owned(),
            produced_by: None,
        },
        // An external relationship carried by a non-present stub: it must surface on the collapsed
        // block through the class read, while the `same_as` edges holding the class together do not.
        linked(canonical, erin, "knows"),
    ]);
    let settings = Settings::default().brief;

    // The present stub is present; the other two ride in the working set as would-be active threads.
    let out = compose_at_epoch(&graph, &settings, &[direct], None, &[canonical, chat]);

    // One block for the identity, headed by the present stub — the bare stubs never get their own.
    assert!(out.contains("## person/rowan@direct"));
    assert!(!out.contains("## person/rowan\n"));
    assert!(!out.contains("## person/rowan@chat"));
    // The two working-set stubs are the same identity as the present one, so nothing is left to render
    // as an active thread and the whole section drops.
    assert!(!out.contains("# Active threads"));
    // The class-wide fact and the present stub's summary each appear exactly once, not once per stub.
    assert_eq!(
        out.matches("maintains an open-source build tool").count(),
        1
    );
    assert_eq!(
        out.matches("An engineer known across two chat platforms.")
            .count(),
        1
    );
    // The external edge surfaces on the collapsed block; the intra-class `same_as` plumbing does not.
    assert!(out.contains("person/rowan@direct → knows → person/erin"));
    assert!(!out.contains("same_as"));
}

#[test]
fn a_hidden_parallel_edge_does_not_shadow_a_visible_one_in_the_brief() {
    // Regression for the relationships dedup order: a far identity (erin, the designated primary of a
    // class holding a platform stub) is reached through two parallel edges — an older public one to the
    // stub and a newer teller-private one to the primary. The block's relationship dedup collapses the
    // pair only *after* the visibility filter, so with the teller absent the public edge survives and
    // the relationship renders once, under the far primary's canonical name. A dedup before the filter
    // would keep the newer private edge, filter it away, and lose the relationship entirely.
    let rowan = MemoryId::generate();
    let erin = MemoryId::generate();
    let erin_stub = MemoryId::generate();
    let maya = MemoryId::generate();
    let (_store, graph) = materialized(vec![
        EventPayload::LinkTypeRegistered {
            name: RelationName::SameAs,
            inverse: RelationName::SameAs,
            from_card: Cardinality::Many,
            to_card: Cardinality::Many,
            symmetric: true,
            reflexive: false,
            description: String::new(),
        },
        register_relation("knows", "known_by"),
        created(rowan, "person/rowan"),
        created(erin, "person/erin"),
        created(erin_stub, "person/9001@testplat"),
        created(maya, "person/maya"),
        EventPayload::link_created(
            erin,
            erin_stub,
            RelationName::SameAs,
            LinkPosture {
                source: LinkSource::Operator,
                told_by: None,
                told_in: None,
                visibility: Visibility::Public,
            },
        ),
        EventPayload::class_primary_designated(erin, true),
        // The older public edge hangs off the stub; the newer parallel edge to the primary is Maya's
        // private aside, hidden from anyone but her.
        linked(rowan, erin_stub, "knows"),
        EventPayload::link_created(
            rowan,
            erin,
            RelationName::new("knows"),
            LinkPosture {
                source: LinkSource::Agent,
                told_by: Some(Teller::Participant(maya)),
                told_in: None,
                visibility: Visibility::PrivateToTeller,
            },
        ),
    ]);
    let settings = Settings::default().brief;

    // Rowan is present and Maya is not, so the private edge is filtered — yet the relationship
    // survives via the public stub edge, rendered once under the far primary's name.
    let out = compose_at_epoch(&graph, &settings, &[rowan], None, &[]);
    assert_eq!(
        out.matches("person/rowan → knows → person/erin").count(),
        1,
        "the parallel edges collapse to one visible relationship: {out}",
    );
    assert!(
        !out.contains("9001@testplat"),
        "the stub snowflake never renders: {out}",
    );
}

#[test]
fn the_present_set_cap_does_not_narrow_the_predicate() {
    // Scenario 21: with the present-set cap set to 1, Dave is present but ranks below the cap (only a
    // name-only entry, no full block). A fact on Marcus (in the cap, rendered) excludes Dave; the
    // exclude must still fire, because the predicate resolves against the full present set — not the
    // capped one. Told by Marcus himself, so the subject-guard does not also suppress it: the exclude
    // is the only thing gating it, isolating the cap-vs-predicate separation.
    let marcus = MemoryId::generate();
    let dave = MemoryId::generate();
    let (_store, graph) = materialized(vec![
        created(marcus, "person/marcus"),
        created(dave, "person/dave"),
        // Marcus has the more recent activity, so he ranks into the cap of 1; Dave falls below it.
        appended(
            marcus,
            2_000,
            "joined the climbing gym",
            Teller::Participant(marcus),
            Visibility::Public,
        ),
        EventPayload::MemoryContentAppended {
            id: marcus,
            entry_id: EntryId::generate(),
            asserted_at: Timestamp::from_millis(2_100),
            occurred_at: None,
            text: "thinking of leaving, keep it from Dave".to_owned(),
            told_by: Teller::Participant(marcus),
            told_in: None,
            visibility: Visibility::Exclude(BTreeSet::from([dave])),
        },
    ]);

    let mut settings = Settings::default().brief;
    settings.present_set_cap = 1;
    let out = compose_at_epoch(&graph, &settings, &[marcus, dave], None, &[]);

    assert!(out.contains("joined the climbing gym")); // Marcus's block renders (in the cap)
    assert!(out.contains("person/dave (present)")); // Dave is present but below the cap (name-only)
    // The exclude fires because Dave is in the full present set, despite ranking below the cap.
    assert!(!out.contains("keep it from Dave"));
}

#[test]
fn the_brief_never_renders_an_entry_id() {
    // The agent-invoked read surfaces lead each entry with its id, but the contextual brief stays lean
    // — its fact lines carry the text and provenance markers only, never the id. This locks the brief's
    // separate rendering path against the id leaking in through a future shared-renderer refactor.
    let priya = MemoryId::generate();
    let entry_id = EntryId::generate();
    let (_store, graph) = materialized(vec![
        created(priya, "person/priya"),
        EventPayload::MemoryContentAppended {
            id: priya,
            entry_id,
            asserted_at: Timestamp::from_millis(1_000),
            occurred_at: None,
            text: "leads the platform migration".to_owned(),
            told_by: Teller::Agent,
            told_in: None,
            visibility: Visibility::Public,
        },
    ]);

    let out = compose_at_epoch(&graph, &Settings::default().brief, &[priya], None, &[]);
    assert!(
        out.contains("leads the platform migration"),
        "the fact renders: {out}"
    );
    assert!(
        !out.contains(&entry_id.0.to_string()),
        "the brief must not render the entry id: {out}"
    );
}
