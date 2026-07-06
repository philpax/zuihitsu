use super::*;

#[tokio::test]
async fn a_created_tag_can_be_applied_and_listed() {
    let h = Harness::new();
    // Create a tag and apply it to a memory in one block (read-your-writes recognizes the pending
    // creation), which commits.
    let seeded = h
        .run(
            r#"
        tags.create("hobbies", "Recreational activities and interests")
        local dave = memory.create(PERSON_DAVE)
        dave:tag("hobbies")
        return "ok"
        "#,
        )
        .await;
    assert!(matches!(seeded, BlockOutcome::Committed { .. }));

    // The tag committed onto Dave.
    let dave = h
        .engine
        .graph
        .lock()
        .memory_by_name(Namespace::Person.with_name("dave"))
        .unwrap()
        .unwrap();
    assert!(dave.tags.contains(&TagName::new("hobbies")));

    // A later block lists the now-committed vocabulary, each entry rendering as a readable line (with
    // its use count) rather than "<table>".
    let listed = h.run(r#"return tags.list()"#).await;
    let BlockOutcome::Committed { result } = listed else {
        panic!("expected commit, got {listed:?}");
    };
    assert!(!result.contains("<table>"), "rendered: {result:?}");
    assert!(
        result.contains("hobbies — Recreational activities and interests (1 use)"),
        "rendered: {result:?}"
    );
}

#[tokio::test]
async fn applying_an_uncreated_tag_is_a_teachable_error() {
    let h = Harness::new();
    // A tag is a described, shared vocabulary — applying one that was never created is teachable, and
    // nothing commits.
    let outcome = h
        .run(
            r#"
        local dave = memory.create(PERSON_DAVE)
        dave:tag("hobbies")
        "#,
        )
        .await;
    match outcome {
        BlockOutcome::Terminated(TerminalCause::Error(message)) => {
            assert!(message.contains("unknown tag"), "message was: {message}");
            assert!(message.contains("tags.create"), "message was: {message}");
        }
        other => panic!("expected a teachable error, got {other:?}"),
    }
    // The whole block was discarded: Dave was not created.
    assert!(
        h.engine
            .graph
            .lock()
            .memory_by_name(Namespace::Person.with_name("dave"))
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn creating_a_duplicate_tag_is_a_teachable_error() {
    let h = Harness::new();
    // Create a tag, which commits.
    let seeded = h.run(r#"tags.create("hobbies", "first purpose")"#).await;
    assert!(matches!(seeded, BlockOutcome::Committed { .. }));

    // Re-creating it is a teachable error — creation forces a fresh purpose, so a collision points at
    // tags.describe to change one instead.
    let outcome = h.run(r#"tags.create("hobbies", "second purpose")"#).await;
    match outcome {
        BlockOutcome::Terminated(TerminalCause::Error(message)) => {
            assert!(message.contains("already exists"), "message was: {message}");
            assert!(message.contains("tags.describe"), "message was: {message}");
        }
        other => panic!("expected a teachable error, got {other:?}"),
    }
}
