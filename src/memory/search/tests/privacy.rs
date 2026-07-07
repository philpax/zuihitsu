use super::*;
async fn a_private_asides_marker_names_its_confidential_room() {
    // Scenario 13's mechanism: an aside told in a #confidential room surfaces flagged with the room
    // and its confidentiality — the cross-context signal the agent reasons over.
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

    // A #confidential context — the #leads room.
    let leads = MemoryId::generate();
    corpus
        .commit(
            1_000,
            vec![EventPayload::memory_created(
                leads,
                Namespace::Context.with_name("leads"),
            )],
        )
        .await;
    corpus.tag(leads, "confidential", 1_000);

    // Erin, in #leads, says something private about Marcus.
    corpus
        .tell_private_in(marcus, "is being managed out", erin, leads, 1_000)
        .await;

    // Erin present, Marcus absent: Marcus surfaces, the marker naming the room and its confidentiality.
    let hits = corpus
        .query_in("is being managed out", None, &[], &[erin], 1_000, 5)
        .await;
    let marcus_hit = hits
        .iter()
        .find(|hit| hit.memory.id == marcus)
        .expect("Marcus surfaces via the aside");
    assert_eq!(
        marcus_hit.marker.as_deref(),
        Some("[teller-private, told by person/erin in #leads (confidential)]")
    );
}

#[tokio::test]
async fn a_stale_description_still_yields_a_legible_snippet() {
    // The legibility guarantee: even when a memory's description is empty (the describer has not
    // caught up), a content match still carries a snippet of what matched — so the hit is
    // triageable rather than a bare name.
    let mut corpus = Corpus::new();
    let devin = corpus
        .add(
            Namespace::Person.with_name("devin"),
            "",
            "owns the rollback and cuts billing over to Stripe on July 20th",
            1_000,
        )
        .await;

    let hits = corpus
        .query_in("cut billing over to Stripe", None, &[], &[], 1_000, 5)
        .await;
    let hit = hits
        .iter()
        .find(|hit| hit.memory.id == devin)
        .expect("Devin surfaces on the content match");
    assert!(
        hit.memory.description.is_empty(),
        "the description is stale/empty, so it cannot carry the hit"
    );
    let snippet = hit
        .snippet
        .as_deref()
        .expect("a matched-content snippet stands in for the missing description");
    assert!(
        snippet.contains("Stripe"),
        "the snippet quotes the matched content: {snippet:?}"
    );
}

#[tokio::test]
async fn a_private_entry_never_appears_in_a_snippet_for_an_excluded_present_set() {
    // The snippet must inherit the same visibility filter as the hit: a private aside's content
    // may never be quoted for a present set that excludes its audience, even though the subject
    // may still surface via public vectors.
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
    corpus
        .tell_private(marcus, "the quarterly review went badly", erin, 1_000)
        .await;

    // Erin absent: the aside's teller is not present, so it never surfaces — and no snippet on any
    // hit may quote its content.
    let hits = corpus
        .query_in(
            "the quarterly review went badly",
            None,
            &[],
            &[marcus],
            1_000,
            5,
        )
        .await;
    assert!(
        hits.iter().all(|hit| hit
            .snippet
            .as_deref()
            .is_none_or(|snippet| !snippet.contains("quarterly review"))),
        "a private aside leaked into a snippet: {hits:?}"
    );

    // Positive control: with Erin present the aside surfaces, and its snippet is legible.
    let hits = corpus
        .query_in(
            "the quarterly review went badly",
            None,
            &[],
            &[erin],
            1_000,
            5,
        )
        .await;
    let marcus_hit = hits
        .iter()
        .find(|hit| hit.memory.id == marcus)
        .expect("Marcus surfaces via the aside");
    assert!(
        marcus_hit
            .snippet
            .as_deref()
            .expect("the surviving aside carries a snippet")
            .contains("quarterly review"),
        "the surfaced aside's snippet quotes its content: {marcus_hit:?}"
    );
}
