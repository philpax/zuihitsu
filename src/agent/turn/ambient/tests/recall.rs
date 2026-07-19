//! Tests for the recall orchestration: salience, exclusion, merged-class collapse, the score
//! threshold, the hit cap, and the disabled switch.

use std::collections::HashSet;

use super::{corpus, merged_rowan, topic};
use crate::{agent::turn::ambient::ambient_recall, ids::MemoryId, settings::AmbientSettings};

#[test]
fn a_salient_content_hit_surfaces() {
    let bonsai = MemoryId::generate();
    let graph = corpus(topic(
        bonsai,
        "bonsai",
        "A schema-migration tool Erin built; it versions and applies database migrations.",
    ));
    let hint = ambient_recall(
        &graph,
        &AmbientSettings::default(),
        "What do you think of bonsai?",
        &HashSet::new(),
        true,
        true,
    )
    .unwrap()
    .expect("a salient hit surfaces");
    assert_eq!(hint.hits.len(), 1);
    assert_eq!(hint.hits[0].memory, bonsai);
    assert!(hint.message.contains("topic/bonsai"));
}

#[test]
fn a_message_with_no_salient_term_surfaces_nothing() {
    let bonsai = MemoryId::generate();
    let graph = corpus(topic(bonsai, "bonsai", "A schema-migration tool."));
    let hint = ambient_recall(
        &graph,
        &AmbientSettings::default(),
        "Thanks, talk soon!",
        &HashSet::new(),
        true,
        true,
    )
    .unwrap();
    assert!(hint.is_none(), "no query term matches the memory");
}

#[test]
fn a_brief_excluded_memory_is_not_hinted() {
    let bonsai = MemoryId::generate();
    let graph = corpus(topic(
        bonsai,
        "bonsai",
        "A schema-migration tool Erin built.",
    ));
    let mut exclude = HashSet::new();
    exclude.insert(bonsai);
    let hint = ambient_recall(
        &graph,
        &AmbientSettings::default(),
        "What do you think of bonsai?",
        &exclude,
        true,
        true,
    )
    .unwrap();
    assert!(hint.is_none(), "an excluded memory is dropped");
}

#[test]
fn a_merged_class_surfaces_once_under_its_primary() {
    // Both stubs match the inbound text, but the class collapses to one hint line, under its
    // primary, rather than naming the identity twice.
    let (graph, direct, chat) = merged_rowan();
    let hint = ambient_recall(
        &graph,
        &AmbientSettings::default(),
        "Any news on the kelp survey?",
        &HashSet::new(),
        true,
        true,
    )
    .unwrap()
    .expect("the merged class surfaces a hint");
    assert_eq!(hint.hits.len(), 1, "the class surfaces as one hit");
    assert_eq!(
        hint.hits[0].memory,
        direct.min(chat),
        "the hit is the class primary"
    );
    let lines = hint.message.lines().filter(|l| l.starts_with("- ")).count();
    assert_eq!(lines, 1, "one hint line for the class: {}", hint.message);
}

#[test]
fn excluding_one_member_suppresses_the_whole_class() {
    // Excluding a non-primary member (say the frozen brief surfaced it) resolves to the class
    // primary, so the whole identity is suppressed rather than surfacing under its other stub.
    let (graph, direct, chat) = merged_rowan();
    let mut exclude = HashSet::new();
    exclude.insert(direct.max(chat));
    let hint = ambient_recall(
        &graph,
        &AmbientSettings::default(),
        "Any news on the kelp survey?",
        &exclude,
        true,
        true,
    )
    .unwrap();
    assert!(
        hint.is_none(),
        "excluding one member excludes the whole class"
    );
}

#[test]
fn the_threshold_filters_weak_matches() {
    let bonsai = MemoryId::generate();
    let graph = corpus(topic(
        bonsai,
        "bonsai",
        "A schema-migration tool Erin built.",
    ));
    // Demanding an unreachably strong match (bm25 is bounded near zero on the weak side, and no
    // real match reaches -1000) filters every hit, so the salient bonsai match is dropped.
    let strict = AmbientSettings {
        min_score: -1_000.0,
        ..AmbientSettings::default()
    };
    let hint = ambient_recall(
        &graph,
        &strict,
        "What do you think of bonsai?",
        &HashSet::new(),
        true,
        true,
    )
    .unwrap();
    assert!(
        hint.is_none(),
        "no hit is strong enough for the strict ceiling"
    );
}

#[test]
fn the_cap_bounds_the_hits() {
    let mut payloads = Vec::new();
    for i in 0..5 {
        payloads.extend(topic(
            MemoryId::generate(),
            &format!("migration{i}"),
            "A database migration tool for schema migration work.",
        ));
    }
    let graph = corpus(payloads);
    let capped = AmbientSettings {
        max_hits: 2,
        ..AmbientSettings::default()
    };
    let hint = ambient_recall(
        &graph,
        &capped,
        "database migration tool",
        &HashSet::new(),
        true,
        true,
    )
    .unwrap()
    .expect("several memories match");
    assert_eq!(hint.hits.len(), 2, "the cap bounds the surfaced hits");
}

#[test]
fn disabled_surfaces_nothing() {
    let bonsai = MemoryId::generate();
    let graph = corpus(topic(
        bonsai,
        "bonsai",
        "A schema-migration tool Erin built.",
    ));
    let off = AmbientSettings {
        enabled: false,
        ..AmbientSettings::default()
    };
    assert!(
        ambient_recall(
            &graph,
            &off,
            "What do you think of bonsai?",
            &HashSet::new(),
            true,
            true,
        )
        .unwrap()
        .is_none()
    );
}
