//! Lua block-transactionality tests: a block's writes commit atomically and project to the graph;
//! reads see the block's own pending writes; scratchpad globals persist across the session's
//! blocks; and an abort or runtime error discards the buffer while recording the terminal cause
//! (spec §Lua API → block transactionality).

#![cfg(feature = "lua")]

mod common;

use common::Harness;
use zuihitsu::{
    BlockOutcome, Clock, ConversationLocator, Graph, ManualClock, MemoryId, MemoryName,
    MemoryStore, Seq, Session, Store, Teller, TerminalCause, Timestamp, TurnId, Visibility,
    event::EventPayload, resolve_or_mint_conversation,
};

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
    assert_eq!(h.graph.entries_local(dave.id).unwrap().len(), 2);
}

#[test]
fn append_carries_teller_context_and_default_visibility() {
    let mut store = MemoryStore::new();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let mut graph = Graph::open_in_memory().unwrap();

    // A room (with its eagerly-minted context memory), the subject, and the speaker.
    let conversation = resolve_or_mint_conversation(
        &mut store,
        &clock,
        &graph,
        &ConversationLocator::new("discord", "leads"),
    )
    .unwrap();
    let phil = MemoryId::generate();
    let erin = MemoryId::generate();
    store
        .append(
            clock.now(),
            vec![
                EventPayload::MemoryCreated {
                    id: phil,
                    name: MemoryName::new("person/phil"),
                },
                EventPayload::MemoryCreated {
                    id: erin,
                    name: MemoryName::new("person/erin"),
                },
            ],
        )
        .unwrap();
    graph.materialize_from(&store).unwrap();
    let context = graph
        .context_for_conversation(conversation)
        .unwrap()
        .unwrap();
    let session = Session::new(conversation);

    let exec = |store: &mut MemoryStore, graph: &mut Graph, script: &str| {
        session
            .execute(
                store,
                graph,
                &clock,
                Teller::Participant(erin),
                TurnId::generate(),
                script,
            )
            .unwrap()
    };

    // Erin, in the room, relays something about Phil: attributed to her, told in this context, and
    // defaulted private to its teller because the subject (Phil) is not the teller.
    exec(
        &mut store,
        &mut graph,
        r#"memory.get("person/phil"):append("is being managed out")"#,
    );
    // `by_agent` records the agent's own observation; `visibility = "public"` forces public.
    exec(
        &mut store,
        &mut graph,
        r#"memory.get("person/phil"):append("seems stressed", { by_agent = true })"#,
    );
    exec(
        &mut store,
        &mut graph,
        r#"memory.get("person/phil"):append("got promoted", { visibility = "public" })"#,
    );

    let entries = graph.entries_local(phil).unwrap();
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].told_by, Teller::Participant(erin));
    assert_eq!(entries[0].told_in, Some(context));
    assert_eq!(entries[0].visibility, Visibility::PrivateToTeller);
    assert_eq!(entries[1].told_by, Teller::Agent);
    assert_eq!(entries[1].visibility, Visibility::Public);
    assert_eq!(entries[2].told_by, Teller::Participant(erin));
    assert_eq!(entries[2].visibility, Visibility::Public); // forced, despite the subject mismatch

    // context.current() resolves to this room's context memory.
    exec(
        &mut store,
        &mut graph,
        r#"context.current():append("kept in confidence", { by_agent = true })"#,
    );
    let context_entries = graph.entries_local(context).unwrap();
    assert_eq!(context_entries.len(), 1);
    assert_eq!(context_entries[0].text, "kept in confidence");
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
