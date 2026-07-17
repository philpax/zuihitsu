//! Contextual-brief composition tests (spec appendix scenarios 2, 14, 21 — the deterministic
//! `[brief]`/`[predicate]` surface). Each builds a materialized graph, composes a brief for a
//! present set, and asserts a fact is present or absent — model-free, because composition is
//! deterministic.
use crate::{
    brief::{self, Brief, BriefFact, BriefRelationship, BriefRequest},
    event::{Cardinality, EventPayload, EventSource, LinkSource, Teller, Visibility},
    graph::Graph,
    ids::{EntryId, MemoryId, MemoryName},
    settings::{BriefSettings, Settings},
    store::{MemoryStore, Store},
    time::{CivilDate, TemporalRef, Timestamp},
    vocabulary::{RelationName, TagName},
};

/// Compose a brief at the epoch (these deterministic tests don't exercise the time-relative
/// `<upcoming/>` window unless they plant a future occurrence, so a fixed `now` keeps them stable).
fn compose_at_epoch(
    graph: &Graph,
    settings: &BriefSettings,
    present_set: &[MemoryId],
    current_context: Option<MemoryId>,
    working_set: &[MemoryId],
) -> String {
    brief::compose(
        graph,
        settings,
        &BriefRequest {
            present_set,
            current_context,
            working_set,
            now: Timestamp::from_millis(0),
        },
    )
    .unwrap()
}

/// A content append carrying an `occurred_at` (the `appended` helper below leaves it `None`).
fn appended_at(
    id: MemoryId,
    occurred_at: TemporalRef,
    text: &str,
    told_by: Teller,
    visibility: Visibility,
) -> EventPayload {
    EventPayload::MemoryContentAppended {
        id,
        entry_id: EntryId::generate(),
        asserted_at: Timestamp::from_millis(0),
        occurred_at: Some(occurred_at),
        text: text.to_owned(),
        told_by,
        told_in: None,
        visibility,
    }
}

/// Build a store, append `payloads`, and materialize a fresh in-memory graph from them.
fn materialized(payloads: Vec<EventPayload>) -> (MemoryStore, Graph) {
    let mut store = MemoryStore::new();
    store
        .append(Timestamp::from_millis(1_000), EventSource::Agent, payloads)
        .unwrap();
    let mut graph = Graph::open_in_memory().unwrap();
    graph.materialize_from(&store).unwrap();
    (store, graph)
}

fn created(id: MemoryId, name: &str) -> EventPayload {
    EventPayload::memory_created(id, MemoryName::new(name))
}

fn appended(
    id: MemoryId,
    at_ms: i64,
    text: &str,
    told_by: Teller,
    visibility: Visibility,
) -> EventPayload {
    EventPayload::MemoryContentAppended {
        id,
        entry_id: EntryId::generate(),
        asserted_at: Timestamp::from_millis(at_ms),
        occurred_at: None,
        text: text.to_owned(),
        told_by,
        told_in: None,
        visibility,
    }
}

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
            LinkSource::Operator,
            None,
            None,
            Visibility::Public,
        ),
        EventPayload::link_created(
            chat,
            canonical,
            RelationName::SameAs,
            LinkSource::Operator,
            None,
            None,
            Visibility::Public,
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
            visibility: Visibility::Exclude(vec![dave]),
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
fn upcoming_block_lists_near_future_items_within_the_window() {
    // now = epoch (day 0). The dentist on day 3 falls in the default 7-day window; the far review on
    // day 30 does not.
    let dentist = MemoryId::generate();
    let far = MemoryId::generate();
    let (_store, graph) = materialized(vec![
        created(dentist, "event/dentist"),
        appended_at(
            dentist,
            TemporalRef::Day(CivilDate("1970-01-04".into())),
            "cleaning",
            Teller::Agent,
            Visibility::Public,
        ),
        created(far, "event/far"),
        appended_at(
            far,
            TemporalRef::Day(CivilDate("1970-01-31".into())),
            "annual review",
            Teller::Agent,
            Visibility::Public,
        ),
    ]);
    let out = compose_at_epoch(&graph, &Settings::default().brief, &[], None, &[]);
    assert!(out.contains("# Upcoming"));
    assert!(out.contains("cleaning"));
    assert!(!out.contains("annual review")); // beyond the 7-day window
}

#[test]
fn the_structured_join_brief_projects_to_the_frozen_markup() {
    // A representative participant brief — a summary, a public fact, an attributed fact carrying a
    // `[via …]` provenance marker, and a relationship — assembled as a `Brief` and rendered. The
    // structured parts are pinned, and the rendered markup is pinned against the exact text the
    // string composer produces, so the projection stays byte-identical to what the agent's prompt
    // reads (and a later change that drifts either apart goes red).
    let priya = MemoryId::generate();
    let erin = MemoryId::generate();
    let (_store, graph) = materialized(vec![
        EventPayload::LinkTypeRegistered {
            name: RelationName::new("knows"),
            inverse: RelationName::new("known_by"),
            from_card: Cardinality::Many,
            to_card: Cardinality::Many,
            symmetric: false,
            reflexive: false,
            description: String::new(),
        },
        created(priya, "person/priya"),
        created(erin, "person/erin"),
        EventPayload::memory_description_regenerated(
            priya,
            "Priya, staff engineer on the platform team",
            None,
        ),
        appended(
            priya,
            1_000,
            "leads the platform migration",
            Teller::Agent,
            Visibility::Public,
        ),
        appended(
            priya,
            1_100,
            "weighing an offer from a competitor",
            Teller::Participant(erin),
            Visibility::Attributed,
        ),
        EventPayload::link_created(
            priya,
            erin,
            RelationName::new("knows"),
            LinkSource::Agent,
            None,
            None,
            Visibility::Public,
        ),
    ]);
    let settings = Settings::default().brief;
    // The join present set includes the joiner (Priya): her attributed fact still surfaces (an
    // attributed entry is visible to anyone), carrying its `[via …]` marker.
    let present_set = [priya, erin];

    let brief = brief::compose_participant_brief(
        &graph,
        priya,
        &present_set,
        &settings,
        Timestamp::from_millis(0),
    )
    .unwrap()
    .expect("Priya is a known memory, so her brief is composed");

    assert_eq!(
        brief,
        Brief {
            subject: MemoryName::new("person/priya"),
            summary: Some("Priya, staff engineer on the platform team".to_owned()),
            recent_facts: vec![
                BriefFact {
                    text: "leads the platform migration".to_owned(),
                    markers: vec![],
                },
                BriefFact {
                    text: "weighing an offer from a competitor".to_owned(),
                    markers: vec!["[via person/erin]".to_owned()],
                },
            ],
            relationships: vec![BriefRelationship {
                relation: RelationName::new("knows"),
                source: MemoryName::new("person/priya"),
                target: MemoryName::new("person/erin"),
                marker: None,
            }],
        }
    );

    let expected = "\
## person/priya
<summary>Priya, staff engineer on the platform team</summary>
<recent_facts>
- leads the platform migration
- weighing an offer from a competitor [via person/erin]
</recent_facts>
<relationships>
- person/priya → knows → person/erin
</relationships>
";
    assert_eq!(brief.render(), expected);
    // The projection is exactly what the string composer produces — the agent-facing format.
    assert_eq!(
        brief.render(),
        brief::compose_participant(
            &graph,
            priya,
            &present_set,
            &settings,
            Timestamp::from_millis(0)
        )
        .unwrap()
    );
}

#[test]
fn upcoming_respects_the_subject_guard() {
    // A private aside about Marcus with a near-future occurrence, told by Erin: visible in <upcoming/>
    // while only Erin is present, suppressed once Marcus (its subject) is present.
    let marcus = MemoryId::generate();
    let erin = MemoryId::generate();
    let (_store, graph) = materialized(vec![
        created(marcus, "person/marcus"),
        created(erin, "person/erin"),
        appended_at(
            marcus,
            TemporalRef::Day(CivilDate("1970-01-04".into())),
            "farewell lunch",
            Teller::Participant(erin),
            Visibility::PrivateToTeller,
        ),
    ]);
    let settings = Settings::default().brief;
    let only_erin = compose_at_epoch(&graph, &settings, &[erin], None, &[]);
    assert!(only_erin.contains("farewell lunch"));
    let with_marcus = compose_at_epoch(&graph, &settings, &[erin, marcus], None, &[]);
    assert!(!with_marcus.contains("farewell lunch"));
}

/// Register a relation `name`/`inverse` (both `Many`, asymmetric) so a link can be created under it.
fn register_relation(name: &str, inverse: &str) -> EventPayload {
    EventPayload::LinkTypeRegistered {
        name: RelationName::new(name),
        inverse: RelationName::new(inverse),
        from_card: Cardinality::Many,
        to_card: Cardinality::Many,
        symmetric: false,
        reflexive: false,
        description: String::new(),
    }
}

/// A public `relation` link from `from` to `to`.
fn linked(from: MemoryId, to: MemoryId, relation: &str) -> EventPayload {
    EventPayload::link_created(
        from,
        to,
        RelationName::new(relation),
        LinkSource::Agent,
        None,
        None,
        Visibility::Public,
    )
}

/// The relationship lines of a rendered brief, in order — the `- {relation}: …` bullets under
/// `<relationships>`, so a test can assert the ranking without pinning the whole block.
fn relationship_lines(rendered: &str) -> Vec<String> {
    rendered
        .lines()
        .skip_while(|line| *line != "<relationships>")
        .skip(1)
        .take_while(|line| *line != "</relationships>")
        .map(str::to_owned)
        .collect()
}

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
