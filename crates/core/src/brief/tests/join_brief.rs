//! The structured join brief projects to the frozen markup: a `Brief` assembled from a summary, a
//! public fact, an attributed fact carrying its `[via …]` provenance marker, and a relationship
//! renders to the exact agent-facing text the string composer produces.
use super::{appended, created, materialized};
use crate::{
    brief::{self, Brief, BriefFact, BriefRelationship},
    event::{Cardinality, EventPayload, LinkPosture, LinkSource, Teller, Visibility},
    ids::{MemoryId, MemoryName},
    settings::Settings,
    time::Timestamp,
    vocabulary::RelationName,
};

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
            LinkPosture {
                source: LinkSource::Agent,
                told_by: None,
                told_in: None,
                visibility: Visibility::Public,
            },
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
