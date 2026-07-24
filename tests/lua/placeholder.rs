//! The free-text placeholder guard: a script that writes a string-format placeholder (`{content}`)
//! inside a plain quoted string handed to the API records the literal braces instead of a value, so
//! the guard rejects it at the argument boundary with a teachable error pointing at the backtick
//! string that interpolates. These tests cover the guarded free-text surfaces (entry text, memory
//! names, search queries, tag purposes) and confirm that backtick interpolation still commits the
//! rendered value while benign braces pass untouched.

use crate::{BlockOutcome, Harness, TerminalCause};

#[tokio::test]
async fn append_with_a_literal_placeholder_is_a_teachable_error() {
    let h = Harness::new();
    h.run(r#"memory.create(TOPIC_CLIMBING)"#).await;
    let outcome = h
        .run(
            r#"local m = memory.get(TOPIC_CLIMBING)
               m:append("Full text: {content}")
               return "unreached""#,
        )
        .await;
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = outcome else {
        panic!("expected a teachable error, got {outcome:?}");
    };
    assert!(message.contains("{content}"), "{message}");
    assert!(message.contains("backtick"), "{message}");
}

#[tokio::test]
async fn create_with_a_placeholder_name_is_a_teachable_error() {
    let h = Harness::new();
    let outcome = h.run(r#"return memory.create("person/{name}")"#).await;
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = outcome else {
        panic!("expected a teachable error, got {outcome:?}");
    };
    assert!(message.contains("{name}"), "{message}");
    assert!(message.contains("backtick"), "{message}");
}

#[tokio::test]
async fn search_with_a_placeholder_query_is_a_teachable_error() {
    let h = Harness::new();
    let outcome = h.run(r#"return memory.search("recent {topic}")"#).await;
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = outcome else {
        panic!("expected a teachable error, got {outcome:?}");
    };
    assert!(message.contains("{topic}"), "{message}");
    assert!(message.contains("backtick"), "{message}");
}

#[tokio::test]
async fn rename_to_a_placeholder_is_a_teachable_error() {
    let h = Harness::new();
    h.run(r#"memory.create(TOPIC_CLIMBING)"#).await;
    let outcome = h
        .run(
            r#"local m = memory.get(TOPIC_CLIMBING)
               m:rename("topic/{new}")
               return "unreached""#,
        )
        .await;
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = outcome else {
        panic!("expected a teachable error, got {outcome:?}");
    };
    assert!(message.contains("{new}"), "{message}");
    assert!(message.contains("backtick"), "{message}");
}

#[tokio::test]
async fn tag_creation_with_a_placeholder_purpose_is_a_teachable_error() {
    let h = Harness::new();
    let outcome = h
        .run(r#"tags.create("mood", "tracks {state}") return "unreached""#)
        .await;
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = outcome else {
        panic!("expected a teachable error, got {outcome:?}");
    };
    assert!(message.contains("{state}"), "{message}");
    assert!(message.contains("backtick"), "{message}");
}

#[tokio::test]
async fn get_with_a_placeholder_name_is_a_teachable_error() {
    // A placeholder name in `memory.get` errors teachably rather than silently returning nil.
    let h = Harness::new();
    let outcome = h.run(r#"return memory.get("{name}")"#).await;
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = outcome else {
        panic!("expected a teachable error, got {outcome:?}");
    };
    assert!(message.contains("{name}"), "{message}");
    assert!(message.contains("backtick"), "{message}");
}

#[tokio::test]
async fn backtick_interpolation_commits_the_rendered_value() {
    let h = Harness::new();
    let outcome = h
        .run(
            r#"
        local m = memory.get_or_create("topic/notes")
        local content = "hello"
        m:append(`Full text: {content}`)
        return "ok"
        "#,
        )
        .await;
    assert!(
        matches!(outcome, BlockOutcome::Committed { .. }),
        "the backtick string interpolates and commits: {outcome:?}"
    );

    let BlockOutcome::Committed { result } = h
        .run(
            r#"
        local m = memory.get("topic/notes")
        local out = {}
        for _, e in ipairs(m:entries()) do
            table.insert(out, e.text)
        end
        return table.concat(out, " || ")
        "#,
        )
        .await
    else {
        panic!("expected a committed read");
    };
    assert!(
        result.contains("Full text: hello"),
        "the rendered value is stored: {result}"
    );
    assert!(
        !result.contains("{content}"),
        "the literal placeholder must not be stored: {result}"
    );
}

#[tokio::test]
async fn benign_braces_are_accepted() {
    let h = Harness::new();
    let outcome = h
        .run(
            r#"
        local m = memory.get_or_create(TOPIC_CLIMBING)
        m:append("schedule options: { day = \"2026-06-03\" }")
        m:append("an empty table is {}")
        return "ok"
        "#,
        )
        .await;
    assert!(
        matches!(outcome, BlockOutcome::Committed { .. }),
        "braces that are not expression-shaped commit: {outcome:?}"
    );
}
