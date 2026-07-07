//! Relation seeding tests — the seed ontology and the reference-example drift guard.

use std::collections::BTreeSet;

use crate::{
    InstanceFeatures,
    agent::genesis,
    event::EventPayload,
    ids::Seq,
    store::{MemoryStore, Store},
};

use super::{clock, seed};

#[test]
fn genesis_seeds_the_part_of_membership_relation() {
    let mut store = MemoryStore::new();
    genesis::rollout(
        &mut store,
        &clock(),
        &seed(),
        None,
        &InstanceFeatures::default(),
    )
    .unwrap();
    let events = store.read_from(Seq::ZERO).unwrap();

    let part_of = events.iter().find_map(|e| match &e.payload {
        EventPayload::LinkTypeRegistered { name, inverse, .. } if name.as_str() == "part_of" => {
            Some(inverse.as_str().to_owned())
        }
        _ => None,
    });
    assert_eq!(
        part_of.as_deref(),
        Some("contains"),
        "genesis must seed part_of with its contains inverse"
    );
}

#[test]
fn genesis_does_not_seed_learned_social_relations() {
    let mut store = MemoryStore::new();
    genesis::rollout(
        &mut store,
        &clock(),
        &seed(),
        None,
        &InstanceFeatures::default(),
    )
    .unwrap();
    let events = store.read_from(Seq::ZERO).unwrap();

    let seeded = |forward: &str| {
        events.iter().any(|e| {
                matches!(&e.payload, EventPayload::LinkTypeRegistered { name, .. } if name.as_str() == forward)
            })
    };
    assert!(
        !seeded("mentors"),
        "mentorship is learned, not seeded — genesis must not register mentors"
    );
    assert!(
        !seeded("located_at"),
        "venue semantics are learned, not seeded — genesis must not register located_at"
    );
}

#[test]
fn every_reference_link_example_is_a_seeded_relation() {
    let seeded: BTreeSet<String> = super::super::seed_relations()
        .into_iter()
        .flat_map(|relation| {
            [
                relation.name.as_str().to_owned(),
                relation.inverse.as_str().to_owned(),
            ]
        })
        .collect();

    let reference = crate::agent::lua::api_reference(&InstanceFeatures::default());
    let link_entries = [
        "<memory>:link",
        "<memory>:unlink",
        "<memory>:outgoing",
        "<memory>:incoming",
    ];
    for entry in reference
        .iter()
        .filter(|entry| link_entries.contains(&entry.call.as_str()))
    {
        let text = std::iter::once(entry.doc.as_str())
            .chain(entry.params.iter().map(|param| param.doc.as_str()))
            .collect::<Vec<_>>()
            .join(" ");
        for example in relation_examples(&text) {
            assert!(
                seeded.contains(&example),
                "{}: example relation {example:?} is not a seeded relation — a copied example \
                     would crash with \"unknown relation\"",
                entry.call
            );
        }
    }
}

/// Extract the relation labels a reference entry illustrates: the value after an `e.g. "…"`
/// marker, and any label passed to a `:link("…")` / `:outgoing("…")` / `:incoming("…")` /
/// `:unlink("…")` call form in the prose. Every one must resolve to a seeded relation.
fn relation_examples(text: &str) -> Vec<String> {
    let markers = [
        "e.g. \"",
        ":link(\"",
        ":outgoing(\"",
        ":incoming(\"",
        ":unlink(\"",
    ];
    let mut examples = Vec::new();
    for marker in markers {
        let mut rest = text;
        while let Some(start) = rest.find(marker) {
            let after = &rest[start + marker.len()..];
            if let Some(end) = after.find('"') {
                examples.push(after[..end].to_owned());
                rest = &after[end..];
            } else {
                break;
            }
        }
    }
    examples
}
