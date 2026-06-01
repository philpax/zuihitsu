//! Lua block-transactionality tests: a block's writes commit atomically and project to the graph;
//! reads see the block's own pending writes; scratchpad globals persist across the session's
//! blocks; and an abort or runtime error discards the buffer while recording the terminal cause
//! (spec §Lua API → block transactionality).

#![cfg(feature = "lua")]

mod common;

use common::Harness;
use zuihitsu::{BlockOutcome, Seq, Store, TerminalCause, event::EventPayload};

#[test]
fn block_commits_and_projects_with_read_your_writes() {
    let mut h = Harness::new();
    let outcome = h.run(
        r#"
        local dave = memory.create("person/dave", "Met at the climbing gym")
        dave:append("Got a new job at Hooli")
        return dave:entries()
        "#,
    );

    // The block saw its own pending writes (read-your-writes), rendered back as the result.
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit");
    };
    assert!(result.contains("Met at the climbing gym"));
    assert!(result.contains("Got a new job at Hooli"));

    // And they committed and projected to the graph.
    let dave = h.graph.memory_by_name("person/dave").unwrap().unwrap();
    assert_eq!(h.graph.entries(dave.id).unwrap().len(), 2);
}

#[test]
fn committed_memory_is_visible_to_a_later_block() {
    let mut h = Harness::new();
    h.run(r#"memory.create("topic/sourdough", "A naturally leavened bread")"#);
    let outcome = h.run(r#"return memory.get("topic/sourdough"):entries()"#);
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit");
    };
    assert!(result.contains("naturally leavened"));
}

#[test]
fn scratchpad_globals_persist_across_blocks() {
    let mut h = Harness::new();
    h.run("scratch = 41");
    let outcome = h.run("return scratch + 1");
    assert_eq!(
        outcome,
        BlockOutcome::Committed {
            result: "42".to_owned()
        }
    );
}

#[test]
fn abort_discards_the_buffer() {
    let mut h = Harness::new();
    let outcome = h.run(
        r#"
        memory.create("person/ghost", "should not survive")
        block.abort("changed my mind")
        "#,
    );
    assert_eq!(
        outcome,
        BlockOutcome::Terminated(TerminalCause::Aborted("changed my mind".to_owned()))
    );
    // The buffered create was discarded.
    assert!(h.graph.memory_by_name("person/ghost").unwrap().is_none());

    // A LuaExecuted recording the abort is still in the log (the agent saw the outcome).
    let aborted = h.store.read_from(Seq::ZERO).unwrap().into_iter().any(|e| {
        matches!(
            e.payload,
            EventPayload::LuaExecuted {
                terminal_cause: Some(TerminalCause::Aborted(_)),
                ..
            }
        )
    });
    assert!(aborted);
}

#[test]
fn runtime_error_discards_the_buffer_and_records_the_cause() {
    let mut h = Harness::new();
    let outcome = h.run(
        r#"
        memory.create("person/oops", "should not survive")
        error("boom")
        "#,
    );
    assert!(matches!(
        outcome,
        BlockOutcome::Terminated(TerminalCause::Error(_))
    ));
    assert!(h.graph.memory_by_name("person/oops").unwrap().is_none());
}

#[test]
fn lua_executed_records_the_script_result_and_touched_set() {
    let mut h = Harness::new();
    h.run(r#"memory.create("place/sydney", "A harbour city") return "done""#);

    let recorded = h
        .store
        .read_from(Seq::ZERO)
        .unwrap()
        .into_iter()
        .find_map(|e| match e.payload {
            EventPayload::LuaExecuted {
                result, touched, ..
            } => Some((result, touched)),
            _ => None,
        })
        .expect("a LuaExecuted event");
    assert_eq!(recorded.0.as_deref(), Some("done"));
    assert_eq!(recorded.1.len(), 1); // touched the one created memory
}
