//! Argument-shape teachable errors: a wrongly-shaped argument at a Lua API seam — a table where a
//! string was wanted — is reworded from mlua's opaque "error converting Lua table to String" into a
//! message naming the function, the expected shape, what arrived, and the correct one-line call (the
//! `arg` helper at the Lua API boundary). These tests cover a representative set of wrapped seams
//! (`memory.search`, `memory.create`, `mem:append`, `calendar.date`), confirm the correct shape still
//! commits, and confirm Luau's own string/number coercion survives the wrapper.

use crate::{BlockOutcome, Harness, MemoryName, Namespace, TerminalCause};

#[tokio::test]
async fn search_with_a_table_query_is_a_teachable_error() {
    let h = Harness::new();
    let outcome = h.run(r#"return memory.search({})"#).await;
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = outcome else {
        panic!("expected a teachable error, got {outcome:?}");
    };
    assert!(
        message.contains("memory.search: expected a query string, got a table"),
        "{message}"
    );
    assert!(message.contains("memory.search(\"dave\")"), "{message}");
}

#[tokio::test]
async fn search_with_a_correct_query_still_runs() {
    let h = Harness::with_retrieval();
    let seeded = h
        .run(
            r#"
        local dave = memory.create(PERSON_DAVE)
        dave:append("An avid rock climber", { by_agent = true, visibility = "public" })
        return "ok"
        "#,
        )
        .await;
    assert!(matches!(seeded, BlockOutcome::Committed { .. }));
    h.index().await;

    let outcome = h
        .run(
            r#"
        local results = memory.search("An avid rock climber")
        if #results == 0 then return "none" end
        return results[1].name
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert_eq!(
        result,
        MemoryName::from(Namespace::Person.with_name("dave")).as_str()
    );
}

#[tokio::test]
async fn create_with_a_table_name_is_a_teachable_error() {
    let h = Harness::new();
    let outcome = h.run(r#"return memory.create({})"#).await;
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = outcome else {
        panic!("expected a teachable error, got {outcome:?}");
    };
    assert!(
        message.contains("memory.create: expected a memory name string, got a table"),
        "{message}"
    );
    assert!(
        message.contains("memory.create(\"person/dave\")"),
        "{message}"
    );
}

#[tokio::test]
async fn create_still_accepts_a_string_name() {
    let h = Harness::new();
    let outcome = h.run(r#"return memory.create(PERSON_DAVE).name"#).await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert!(
        result.starts_with(MemoryName::from(Namespace::Person.with_name("dave")).as_str()),
        "{result}"
    );
}

#[tokio::test]
async fn append_with_a_table_text_is_a_teachable_error() {
    let h = Harness::new();
    h.run(r#"memory.create(TOPIC_CLIMBING)"#).await;
    let outcome = h
        .run(
            r#"local m = memory.get(TOPIC_CLIMBING)
               m:append({})
               return "unreached""#,
        )
        .await;
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = outcome else {
        panic!("expected a teachable error, got {outcome:?}");
    };
    assert!(
        message.contains("mem:append: expected the entry text as a string, got a table"),
        "{message}"
    );
}

#[tokio::test]
async fn append_still_coerces_a_number_the_way_luau_does() {
    // The wrapper delegates to the real `FromLua`, so Luau's own number-to-string coercion still
    // applies — a number handed where a string is wanted is not falsely refused.
    let h = Harness::new();
    h.run(r#"memory.create(TOPIC_CLIMBING)"#).await;
    let outcome = h
        .run(
            r#"local m = memory.get(TOPIC_CLIMBING)
               local e = m:append(42)
               return e.text"#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert!(result.starts_with("42"), "{result}");
}

#[tokio::test]
async fn calendar_date_with_a_table_is_a_teachable_error() {
    let h = Harness::new();
    let outcome = h.run(r#"return calendar.date({})"#).await;
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = outcome else {
        panic!("expected a teachable error, got {outcome:?}");
    };
    assert!(
        message.contains("calendar.date: expected a \"YYYY-MM-DD\" date string, got a table"),
        "{message}"
    );
    assert!(
        message.contains("calendar.date(\"2026-06-03\")"),
        "{message}"
    );
}

#[tokio::test]
async fn calendar_date_still_accepts_a_string() {
    let h = Harness::new();
    let outcome = h.run(r#"return calendar.date("2026-06-03").day"#).await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert_eq!(result, "2026-06-03");
}
