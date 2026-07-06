use zuihitsu::{
    Cardinality, Event, EventPayload, LinkSource, MemoryId, MemoryName, RelationName, Seq,
    Timestamp,
};

use super::{project_relations, render_locations, render_shapes};
use crate::package::{
    Aggregate, Bar, Category, EvalPackage, RunMeta, RunMetrics, RunRecord, ScenarioMeta,
    ScenarioReport, Stat, TokenStat,
};

fn event(payload: EventPayload) -> Event {
    Event {
        seq: Seq::ZERO,
        recorded_at: Timestamp::from_millis(0),
        payload,
    }
}

fn registered(name: &str, inverse: &str) -> EventPayload {
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

fn created(id: MemoryId, name: &str) -> EventPayload {
    EventPayload::memory_created(id, MemoryName::new(name))
}

fn linked(from: MemoryId, to: MemoryId, relation: &str) -> EventPayload {
    EventPayload::LinkCreated {
        from,
        to,
        relation: RelationName::new(relation),
        source: LinkSource::Inferred,
        told_by: None,
    }
}

fn genesis() -> EventPayload {
    EventPayload::genesis_completed("hash", Default::default())
}

/// A one-run scenario carrying `events` verbatim — the projection reads only the name, run index,
/// and events, so the aggregate is filler.
fn scenario(name: &str, run_events: Vec<Event>) -> ScenarioReport {
    let stat = Stat {
        p50: 0.0,
        p95: 0.0,
        mean: 0.0,
    };
    ScenarioReport {
        meta: ScenarioMeta {
            name: name.to_owned(),
            category: Category::Relations,
            description: "synthetic".to_owned(),
            bar: Bar::gating(),
        },
        runs: vec![RunRecord {
            index: 0,
            started_at_ms: 0,
            finished_at_ms: 0,
            events: run_events,
            verdicts: Vec::new(),
            metrics: RunMetrics::default(),
        }],
        aggregate: Aggregate {
            runs: 1,
            rate: 1.0,
            gating_passed: true,
            gating_rate: 1.0,
            wall_clock_ms: stat,
            latency_ms: stat,
            tokens: TokenStat {
                prompt_mean: 0.0,
                completion_mean: 0.0,
                total_mean: 0.0,
            },
            steps_mean: 0.0,
        },
    }
}

fn package(scenarios: Vec<ScenarioReport>) -> EvalPackage {
    EvalPackage {
        meta: RunMeta {
            harness_version: "test".to_owned(),
            git_sha: None,
            git_dirty: false,
            model_id: "test-model".to_owned(),
            embedding_model: None,
            scenario_filter: None,
            started_at_ms: 0,
            finished_at_ms: 0,
            runs_per_scenario: 1,
            concurrency: 1,
        },
        scenarios,
    }
}

/// A run that seeds `knows` at genesis, coins `mentored_by` after, and draws a link under each — the
/// canonical shape the projection tabulates.
fn seeded_and_coined_run() -> Vec<Event> {
    let marcus = MemoryId::generate();
    let clara = MemoryId::generate();
    let zephyr = MemoryId::generate();
    vec![
        // Genesis: the seed relation is registered before the completion marker.
        event(registered("knows", "known_by")),
        event(genesis()),
        // The turn mints memories, coins a relation, and links under both.
        event(created(marcus, "person/marcus")),
        event(created(clara, "person/clara")),
        event(created(zephyr, "topic/zephyr")),
        event(linked(marcus, clara, "knows")),
        event(registered("mentored_by", "mentored")),
        event(linked(zephyr, clara, "mentored_by")),
    ]
}

#[test]
fn projection_splits_seeded_from_coined_and_counts_shapes() {
    let pkg = package(vec![scenario("infers_link", seeded_and_coined_run())]);
    let report = project_relations(&pkg, None);

    assert_eq!(report.runs_scanned, 1);
    // Two relations used, both drawn once, most-used ties broken by name (mentored_by before knows).
    assert_eq!(report.vocab.len(), 2);

    let knows = report
        .vocab
        .iter()
        .find(|row| row.relation == "knows")
        .expect("knows is in the vocabulary");
    assert!(knows.seeded, "knows was registered before GenesisCompleted");
    assert_eq!(knows.uses, 1);
    assert_eq!(knows.shapes, vec![("person→person".to_owned(), 1)]);

    let mentored = report
        .vocab
        .iter()
        .find(|row| row.relation == "mentored_by")
        .expect("mentored_by is in the vocabulary");
    assert!(
        !mentored.seeded,
        "mentored_by was registered after GenesisCompleted"
    );
    assert_eq!(mentored.uses, 1);
    assert_eq!(mentored.shapes, vec![("topic→person".to_owned(), 1)]);

    // The coinage section holds exactly the post-genesis registration, with its inverse and location.
    assert_eq!(report.coinages.len(), 1);
    let coinage = &report.coinages[0];
    assert_eq!(coinage.relation, "mentored_by");
    assert_eq!(coinage.inverse, "mentored");
    assert_eq!(coinage.uses, 1);
    assert_eq!(coinage.coined_in_runs, 1);
    assert_eq!(coinage.locations, vec![("infers_link".to_owned(), 0)]);
}

#[test]
fn shapes_accumulate_across_links_and_runs_sorted_by_frequency() {
    let build = || {
        let a = MemoryId::generate();
        let b = MemoryId::generate();
        let c = MemoryId::generate();
        vec![
            event(registered("knows", "known_by")),
            event(genesis()),
            event(created(a, "person/a")),
            event(created(b, "person/b")),
            event(created(c, "event/c")),
            event(linked(a, b, "knows")),
            event(linked(b, a, "knows")),
            event(linked(a, c, "knows")),
        ]
    };
    let pkg = package(vec![
        scenario("scenario_one", build()),
        scenario("scenario_two", build()),
    ]);
    let report = project_relations(&pkg, None);

    assert_eq!(report.runs_scanned, 2);
    let knows = &report.vocab[0];
    assert_eq!(knows.relation, "knows");
    assert_eq!(knows.uses, 6);
    // person→person twice per run (4 total) outranks person→event (2 total).
    assert_eq!(
        knows.shapes,
        vec![
            ("person→person".to_owned(), 4),
            ("person→event".to_owned(), 2),
        ]
    );
}

#[test]
fn the_scenario_filter_narrows_the_scan() {
    let pkg = package(vec![
        scenario("merges_two_stubs", seeded_and_coined_run()),
        scenario("recalls_a_fact", seeded_and_coined_run()),
    ]);
    let report = project_relations(&pkg, Some("merges"));
    assert_eq!(report.runs_scanned, 1);
    assert_eq!(
        report.coinages[0].locations,
        vec![("merges_two_stubs".to_owned(), 0)]
    );
}

#[test]
fn an_unresolvable_endpoint_renders_as_an_id_stub() {
    let ghost = MemoryId::generate();
    let marcus = MemoryId::generate();
    // No `MemoryCreated` for `ghost`, so it cannot resolve to a name.
    let events = vec![
        event(genesis()),
        event(created(marcus, "person/marcus")),
        event(linked(ghost, marcus, "knows")),
    ];
    let pkg = package(vec![scenario("orphan_link", events)]);
    let report = project_relations(&pkg, None);
    let stub: String = ghost.0.to_string().chars().take(8).collect();
    assert_eq!(report.vocab[0].shapes, vec![(format!("{stub}→person"), 1)]);
}

#[test]
fn render_shapes_caps_and_notes_the_remainder() {
    let shapes: Vec<(String, usize)> = (0..8)
        .map(|index| (format!("ns{index}→person"), 8 - index))
        .collect();
    let rendered = render_shapes(&shapes);
    assert!(rendered.contains("ns0→person ×8"));
    assert!(rendered.ends_with("+2 more"));
}

#[test]
fn render_locations_groups_run_indices_under_each_scenario() {
    let locations = vec![
        ("beta".to_owned(), 1),
        ("alpha".to_owned(), 2),
        ("alpha".to_owned(), 0),
    ];
    assert_eq!(render_locations(&locations), "alpha #2, #0; beta #1");
}
