use super::*;

#[tokio::test]
async fn propose_merge_records_a_pending_proposal_and_merges_nothing() {
    // The agent proposes two stubs are one person. The proposal is inert: it buffers a MergeProposed
    // that pends for the operator, authoring no same_as and leaving the stubs in separate classes —
    // nothing merges without the operator (spec §Cross-platform identity).
    let h = Harness::new();
    h.run(
        r#"
        local a = memory.create(PERSON_DAVE_FORUM)
        a:append("Off sick the first week of March", { visibility = "private" })
        local b = memory.create(PERSON_DAVE_CHAT)
        b:append("Out sick the week of March 3rd", { visibility = "private" })
        a:propose_merge(b)
        return "ok"
        "#,
    )
    .await;

    let graph = h.engine.graph.lock();
    let a = graph
        .memory_by_name(Namespace::Person.with_name("dave-forum"))
        .unwrap()
        .unwrap();
    let b = graph
        .memory_by_name(Namespace::Person.with_name("dave-chat"))
        .unwrap()
        .unwrap();
    assert!(
        !graph.class_members(a.id).unwrap().contains(&b.id),
        "a proposal must merge nothing on its own — the stubs stay in separate classes"
    );
    drop(graph);
    assert!(
        h.events()
            .iter()
            .any(|e| matches!(&e.payload, EventPayload::MergeProposed { .. })),
        "the proposal should be recorded as a pending MergeProposed for the operator"
    );
}

#[tokio::test]
async fn a_proposals_rationale_rides_the_proposed_event() {
    // The rationale the agent states with propose_merge rides the MergeProposed event as the proposer's
    // stated grounds, so the operator can weigh the claim against the two stubs' persisted entries.
    let h = Harness::new();
    h.run(
        r#"
        local a = memory.create(PERSON_DAVE_FORUM)
        a:append("At the Reykjavik conference in June", { visibility = "public" })
        local b = memory.create(PERSON_DAVE_CHAT)
        b:append("Was on a research trip to Iceland", { visibility = "public" })
        a:propose_merge(b, { rationale = "Both mention the same volcanology trip and the same wedding." })
        return "ok"
        "#,
    )
    .await;

    assert!(
        h.events().iter().any(|e| matches!(
            &e.payload,
            EventPayload::MergeProposed { rationale: Some(text), .. }
                if text == "Both mention the same volcanology trip and the same wedding."
        )),
        "the rationale must ride the MergeProposed event"
    );
}
