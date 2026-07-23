use super::*;
#[tokio::test]
async fn a_hit_carries_the_resolved_occurrence() {
    // The date-legibility guarantee: a scheduled fact's resolved occurrence rides on the hit, so a
    // recall that renders from the result line keeps the *when* — rather than the date surfacing
    // only if the agent separately drills into `entries()`.
    let mut corpus = Corpus::new();
    let ship = TemporalRef::Day(CivilDate("2026-07-17".into()));
    let migration = corpus
        .add_dated(
            Namespace::Event.with_name("billing-migration"),
            "The billing migration",
            "shipping the billing migration on Friday the 17th",
            ship.clone(),
            1_000,
        )
        .await;

    let hits = corpus
        .query_in("shipping the billing migration", None, &[], &[], 1_000, 5)
        .await;
    let hit = hits
        .iter()
        .find(|hit| hit.memory.id == migration)
        .expect("the migration surfaces on the content match");
    assert_eq!(
        hit.occurred_at.as_ref(),
        Some(&ship),
        "the hit carries the resolved occurrence: {hit:?}"
    );
}

#[tokio::test]
async fn an_authored_date_outranks_a_newer_extracted_date_on_a_hit() {
    // Authored is ground truth; extracted is inference. An older authored July date must ride on
    // the hit over a *newer* extracted June date on a sibling entry — the exact shadowing that
    // occurs when a relative phrase like "that weekend" is resolved against the clock and the
    // wrong June range shadows the stated July cutover.
    let mut corpus = Corpus::new();
    let id = MemoryId::generate();
    let authored = EntryId::generate();
    let extracted = EntryId::generate();
    let july = TemporalRef::Day(CivilDate("2026-07-20".into()));
    let june = TemporalRef::Day(CivilDate("2026-06-08".into()));
    // Entry 1 carries the authored July cutover; entry 2 (newer) is appended untimed.
    corpus
        .commit(
            1_000,
            vec![
                EventPayload::memory_created(id, Namespace::Event.with_name("billing-cutover")),
                EventPayload::MemoryContentAppended {
                    id,
                    entry_id: authored,
                    asserted_at: Timestamp::from_millis(1_000),
                    occurred_at: Some(july.clone()),
                    text: "cut billing over to the new Stripe integration".to_owned(),
                    told_by: Teller::Agent,
                    told_in: None,
                    visibility: Visibility::Public,
                },
                EventPayload::MemoryContentAppended {
                    id,
                    entry_id: extracted,
                    asserted_at: Timestamp::from_millis(1_000),
                    occurred_at: None,
                    text: "Devin owns the rollback and makes the call that weekend".to_owned(),
                    told_by: Teller::Agent,
                    told_in: None,
                    visibility: Visibility::Public,
                },
            ],
        )
        .await;
    // The extraction pass later (mis)resolves the second entry to a June date against the clock.
    corpus
        .commit(
            2_000,
            vec![EventPayload::entry_temporal_resolved(
                id,
                extracted,
                Some(june.clone()),
                None,
            )],
        )
        .await;

    let hits = corpus
        .query_in(
            "cut billing over to Stripe rollback",
            None,
            &[],
            &[],
            2_000,
            5,
        )
        .await;
    let hit = hits
        .iter()
        .find(|hit| hit.memory.id == id)
        .expect("the cutover surfaces on the content match");
    assert_eq!(
        hit.occurred_at.as_ref(),
        Some(&july),
        "the authored July date must outrank the newer extracted June date: {hit:?}"
    );
}

#[tokio::test]
async fn an_extracted_date_still_surfaces_when_no_authored_date_exists() {
    // The preference falls back rather than dropping the date: with no authored occurrence in the
    // class, the most recent visible extracted occurrence still rides on the hit.
    let mut corpus = Corpus::new();
    let id = MemoryId::generate();
    let entry = EntryId::generate();
    let june = TemporalRef::Day(CivilDate("2026-06-08".into()));
    corpus
        .commit(
            1_000,
            vec![
                EventPayload::memory_created(id, Namespace::Event.with_name("rollback-call")),
                EventPayload::MemoryContentAppended {
                    id,
                    entry_id: entry,
                    asserted_at: Timestamp::from_millis(1_000),
                    occurred_at: None,
                    text: "Devin makes the rollback call that weekend".to_owned(),
                    told_by: Teller::Agent,
                    told_in: None,
                    visibility: Visibility::Public,
                },
            ],
        )
        .await;
    corpus
        .commit(
            2_000,
            vec![EventPayload::entry_temporal_resolved(
                id,
                entry,
                Some(june.clone()),
                None,
            )],
        )
        .await;

    let hits = corpus
        .query_in("Devin makes the rollback call", None, &[], &[], 2_000, 5)
        .await;
    let hit = hits
        .iter()
        .find(|hit| hit.memory.id == id)
        .expect("the rollback call surfaces on the content match");
    assert_eq!(
        hit.occurred_at.as_ref(),
        Some(&june),
        "the extracted date surfaces when there is no authored one: {hit:?}"
    );
}

#[tokio::test]
async fn a_private_entrys_date_never_leaks_into_a_hit() {
    // The occurrence inherits the snippet's visibility filter: a date on a private aside may never
    // ride on a hit for a present set that excludes its audience, even though the subject may still
    // surface via public vectors.
    let mut corpus = Corpus::new();
    let erin = corpus
        .add(
            Namespace::Person.with_name("erin"),
            "A colleague",
            "We work together",
            1_000,
        )
        .await;
    let marcus = corpus
        .add(
            Namespace::Person.with_name("marcus"),
            "A teammate",
            "On the same team",
            1_000,
        )
        .await;
    // The only dated entry on Marcus is Erin's private aside, so any date on his hit can come only
    // from it — an unambiguous probe for a leak.
    let review = TemporalRef::Day(CivilDate("2026-07-20".into()));
    corpus
        .tell_private_dated(
            marcus,
            "his review is on the 20th",
            erin,
            review.clone(),
            1_000,
        )
        .await;

    // Erin absent: the aside is not visible, so no hit may carry its date.
    let hits = corpus
        .query_in("his review is on the 20th", None, &[], &[marcus], 1_000, 5)
        .await;
    assert!(
        hits.iter().all(|hit| hit.occurred_at.is_none()),
        "a private aside's date leaked onto a hit: {hits:?}"
    );

    // Positive control: with Erin present the aside surfaces, so its date rides on Marcus's hit.
    let hits = corpus
        .query_in("his review is on the 20th", None, &[], &[erin], 1_000, 5)
        .await;
    let marcus_hit = hits
        .iter()
        .find(|hit| hit.memory.id == marcus)
        .expect("Marcus surfaces via the aside");
    assert_eq!(
        marcus_hit.occurred_at.as_ref(),
        Some(&review),
        "the surfaced aside's date rides on the hit: {marcus_hit:?}"
    );
}

#[tokio::test]
async fn a_hit_carries_its_salient_relations_people_first() {
    // The informed-creation surface: a hit for a linked memory passively carries its most salient
    // relations, people first, so a search for the book club shows the cast already on it — the
    // recognition signal that steers a recall toward reuse over a name-guessed duplicate.
    let mut corpus = Corpus::new();
    let club = corpus
        .add(
            Namespace::Event.with_name("book_club"),
            "The monthly book club",
            "we discussed the book",
            1_000,
        )
        .await;
    let maya = corpus
        .add(
            Namespace::Person.with_name("maya"),
            "A reader",
            "reads a lot",
            1_000,
        )
        .await;
    let nadia = corpus
        .add(
            Namespace::Person.with_name("nadia"),
            "A reader",
            "reads a lot",
            1_000,
        )
        .await;
    let venue = corpus
        .add(
            Namespace::Topic.with_name("library"),
            "The venue",
            "meets there",
            1_000,
        )
        .await;
    let snacks = corpus
        .add(
            Namespace::Topic.with_name("snacks"),
            "The snacks",
            "brings snacks",
            1_000,
        )
        .await;

    // Link the two non-person memories first (older rows), then the two people (newest rows). With
    // person-first salience the people float ahead of the more-recent non-person, and the cap of 3
    // elides the last non-person behind a `(+1 more)` note.
    corpus.link(venue, club, "hosts", 1_000).await;
    corpus.link(snacks, club, "supplies", 1_000).await;
    corpus.link(maya, club, "participates_in", 1_000).await;
    corpus.link(nadia, club, "participates_in", 1_000).await;

    let hits = corpus
        .query_in("The monthly book club", None, &[], &[], 1_000, 8)
        .await;
    let hit = hits
        .iter()
        .find(|hit| hit.memory.id == club)
        .expect("the book club surfaces on its description");

    assert_eq!(hit.relations.len(), SALIENCE_CAP);
    assert_eq!(
        hit.more_relations, 1,
        "one salient link elided past the cap"
    );
    let person = Namespace::Person.prefix();
    assert!(
        hit.relations[0].other_name.as_str().starts_with(person)
            && hit.relations[1].other_name.as_str().starts_with(person),
        "people anchor identity, so they come first: {:?}",
        hit.relations
    );
    // The two people participate in the club — the edge runs into the club's class, so it reads as
    // incoming, which the hit line renders with a `←`.
    assert!(
        hit.relations
            .iter()
            .take(2)
            .all(|relation| relation.direction == LinkDirection::Incoming
                && relation.relation == RelationName::new("participates_in")),
    );
    let names: Vec<&str> = hit
        .relations
        .iter()
        .map(|relation| relation.other_name.as_str())
        .collect();
    assert!(names.contains(&MemoryName::from(Namespace::Person.with_name("maya")).as_str()));
    assert!(names.contains(&MemoryName::from(Namespace::Person.with_name("nadia")).as_str()));
    // The third salient link is the most-recently created non-person (snacks over library).
    assert_eq!(
        hit.relations[2].other_name.as_str(),
        MemoryName::from(Namespace::Topic.with_name("snacks")).as_str(),
    );
}

#[tokio::test]
async fn an_unlinked_hit_carries_no_relations() {
    // A memory with no out-of-class links carries no salient relations, so the hit line stays bare
    // rather than trailing an empty `— ` segment.
    let mut corpus = Corpus::new();
    let solo = corpus
        .add(
            Namespace::Topic.with_name("sourdough"),
            "Naturally leavened bread",
            "fed the starter",
            1_000,
        )
        .await;

    let hits = corpus
        .query_in("Naturally leavened bread", None, &[], &[], 1_000, 8)
        .await;
    let hit = hits
        .iter()
        .find(|hit| hit.memory.id == solo)
        .expect("the topic surfaces on its description");
    assert!(hit.relations.is_empty());
    assert_eq!(hit.more_relations, 0);
}

#[test]
fn settings_round_trip_through_the_log() {
    let mut store = MemoryStore::new();
    let seed = SeedSelf {
        agent_name: "Kestrel".to_owned(),
        persona: "A companion.".to_owned(),
        seed_entries: Vec::new(),
    };
    genesis::rollout(
        &mut store,
        &ManualClock::new(Timestamp::from_millis(1)),
        &seed,
        None,
        &InstanceFeatures::default(),
    )
    .unwrap();

    // Genesis seeds the default snapshot, so folding the log back yields exactly Settings::default().
    assert_eq!(Settings::from_store(&store).unwrap(), Settings::default());
}
