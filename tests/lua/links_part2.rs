use super::*;
#[tokio::test]
async fn outgoing_under_an_unregistered_relation_is_a_teachable_error() {
    let h = Harness::new();
    h.run(r#"memory.create(PERSON_DAVE)"#).await;
    let outcome = h
        .run(r#"return memory.get(PERSON_DAVE):outgoing("bogus_rel")"#)
        .await;
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = outcome else {
        panic!("expected a teachable error, got {outcome:?}");
    };
    assert!(message.contains("bogus_rel"), "{message}");
}

#[tokio::test]
async fn a_registered_relation_can_be_linked_and_listed() {
    let h = Harness::new();
    // Register a relation and use it to link two memories in the same block — read-your-writes makes
    // the pending registration visible to mem:link.
    let seeded = h
        .run(
            r#"
        links.register({ name = "mentor_of", inverse = "mentored_by", from_card = "many", to_card = "many" })
        local dave = memory.create(PERSON_DAVE)
        local erin = memory.create(PERSON_ERIN)
        dave:link("mentor_of", erin, { visibility = "public" })
        return "ok"
        "#,
        )
        .await;
    assert!(matches!(seeded, BlockOutcome::Committed { .. }));

    // The edge committed: Erin is a mentor_of-neighbour of Dave.
    let (dave, erin) = {
        let graph = h.engine.graph.lock();
        let dave = graph
            .memory_by_name(Namespace::Person.with_name("dave"))
            .unwrap()
            .unwrap();
        let erin = graph
            .memory_by_name(Namespace::Person.with_name("erin"))
            .unwrap()
            .unwrap();
        (dave.id, erin.id)
    };
    let neighbours = h.engine.graph.lock().outgoing(dave, "mentor_of").unwrap();
    assert!(neighbours.iter().any(|memory| memory.id == erin));

    // A later block lists the now-committed registry and resolves a relation by its inverse label,
    // both rendering readably rather than "<table>".
    let listed = h.run(r#"return links.list()"#).await;
    let BlockOutcome::Committed { result } = listed else {
        panic!("expected commit, got {listed:?}");
    };
    assert!(!result.contains("<table>"), "rendered: {result:?}");
    assert!(
        result.contains("mentor_of / mentored_by — many-to-many"),
        "rendered: {result:?}"
    );

    let got = h.run(r#"return tostring(links.get("mentored_by"))"#).await;
    let BlockOutcome::Committed { result } = got else {
        panic!("expected commit, got {got:?}");
    };
    assert!(
        result.contains("mentor_of / mentored_by"),
        "rendered: {result:?}"
    );
}

#[tokio::test]
async fn a_link_can_be_asserted_under_the_inverse_label() {
    let h = Harness::new();
    // spec §Data model: one relation, two labels. Register mentor_of/mentored_by, then assert the edge
    // under the *inverse* label — it must validate (the inverse resolves to the same relation) and
    // canonicalize to the same stored edge as asserting it forwards.
    let outcome = h
        .run(
            r#"
        links.register({ name = "mentor_of", inverse = "mentored_by", from_card = "many", to_card = "many" })
        local dave = memory.create(PERSON_DAVE)
        local erin = memory.create(PERSON_ERIN)
        erin:link("mentored_by", dave, { visibility = "public" })
        return "ok"
        "#,
        )
        .await;
    assert!(matches!(outcome, BlockOutcome::Committed { .. }));

    // "erin mentored_by dave" is the same canonical edge as "dave mentor_of erin".
    let (dave, erin) = {
        let graph = h.engine.graph.lock();
        (
            graph
                .memory_by_name(Namespace::Person.with_name("dave"))
                .unwrap()
                .unwrap()
                .id,
            graph
                .memory_by_name(Namespace::Person.with_name("erin"))
                .unwrap()
                .unwrap()
                .id,
        )
    };
    let neighbours = h.engine.graph.lock().outgoing(dave, "mentor_of").unwrap();
    assert!(
        neighbours.iter().any(|memory| memory.id == erin),
        "dave should be mentor_of erin"
    );
}

#[tokio::test]
async fn registering_a_relation_with_a_bad_cardinality_is_a_teachable_error() {
    let h = Harness::new();
    // A cardinality must be "one" or "many"; anything else is a teachable error, caught at the block
    // boundary before a bad value reaches the registry.
    let outcome = h
        .run(r#"links.register({ name = "x", inverse = "y", from_card = "lots", to_card = "many" })"#)
        .await;
    match outcome {
        BlockOutcome::Terminated(TerminalCause::Error(message)) => {
            assert!(message.contains("cardinality"), "message was: {message}");
            assert!(
                message.contains("\"one\" or \"many\""),
                "message was: {message}"
            );
        }
        other => panic!("expected a teachable error, got {other:?}"),
    }
}
