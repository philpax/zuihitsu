//! Lua block-transactionality tests: a block's writes commit atomically and project to the graph;
//! reads see the block's own pending writes; scratchpad globals persist across the session's
//! blocks; and an abort or runtime error discards the buffer while recording the terminal cause
//! (spec §Lua API → block transactionality).

#![cfg(feature = "lua")]

mod common;

use common::Harness;
use zuihitsu::{
    Authority, BlockContext, BlockOutcome, Cardinality, Clock, ConversationLocator, Engine, Graph,
    ManualClock, MemoryId, MemoryName, MemoryStore, RelationName, Seq, Session, Store, TagName,
    Teller, TerminalCause, Timestamp, TurnId, Visibility, event::EventPayload,
    resolve_or_mint_conversation,
};

#[test]
fn block_commits_and_projects_with_read_your_writes() {
    let mut h = Harness::new();
    let outcome = h.run(
        r#"
        local dave = memory.create("person/dave")
        dave:append("Met at the climbing gym", { visibility = "public" })
        dave:append("Got a new job at Hooli", { visibility = "public" })
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
                &mut Engine {
                    store,
                    graph,
                    clock: &clock,
                },
                &BlockContext {
                    teller: Teller::Participant(erin),
                    authority: Authority::Platform,
                    turn_id: TurnId::generate(),
                },
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
    // `by_agent` records the agent's own observation about a person, which has no protective default
    // (the aside mechanism keys on a participant teller) — so it must classify the entry explicitly.
    exec(
        &mut store,
        &mut graph,
        r#"memory.get("person/phil"):append("seems stressed", { by_agent = true, visibility = "public" })"#,
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
fn link_flags_a_memory_active_in_the_context_and_unlink_clears_it() {
    let mut store = MemoryStore::new();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let mut graph = Graph::open_in_memory().unwrap();

    // A room (with its context memory), the active_in relation, and a thread memory.
    let conversation = resolve_or_mint_conversation(
        &mut store,
        &clock,
        &graph,
        &ConversationLocator::new("discord", "leads"),
    )
    .unwrap();
    let roadmap = MemoryId::generate();
    store
        .append(
            clock.now(),
            vec![
                EventPayload::LinkTypeRegistered {
                    name: RelationName::new("active_in"),
                    inverse: RelationName::new("has_active"),
                    from_card: Cardinality::Many,
                    to_card: Cardinality::Many,
                    symmetric: false,
                    reflexive: false,
                },
                EventPayload::MemoryCreated {
                    id: roadmap,
                    name: MemoryName::new("topic/roadmap"),
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

    // The agent flags the thread active_in the current context.
    let outcome = session
        .execute(
            &mut Engine {
                store: &mut store,
                graph: &mut graph,
                clock: &clock,
            },
            &BlockContext {
                teller: Teller::Agent,
                authority: Authority::Platform,
                turn_id: TurnId::generate(),
            },
            r#"memory.get("topic/roadmap"):link("active_in", context.current())"#,
        )
        .unwrap();
    assert!(matches!(outcome, BlockOutcome::Committed { .. }));
    // Read back through the has_active inverse: the context now carries the thread.
    let active = graph.outgoing(context, "has_active").unwrap();
    assert!(active.iter().any(|memory| memory.id == roadmap));

    // Unlinking clears it.
    session
        .execute(
            &mut Engine {
                store: &mut store,
                graph: &mut graph,
                clock: &clock,
            },
            &BlockContext {
                teller: Teller::Agent,
                authority: Authority::Platform,
                turn_id: TurnId::generate(),
            },
            r#"memory.get("topic/roadmap"):unlink("active_in", context.current())"#,
        )
        .unwrap();
    assert!(graph.outgoing(context, "has_active").unwrap().is_empty());
}

#[test]
fn a_write_in_a_confidential_room_defaults_private() {
    let mut store = MemoryStore::new();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let mut graph = Graph::open_in_memory().unwrap();

    let conversation = resolve_or_mint_conversation(
        &mut store,
        &clock,
        &graph,
        &ConversationLocator::new("discord", "leads"),
    )
    .unwrap();
    graph.materialize_from(&store).unwrap();
    let context = graph
        .context_for_conversation(conversation)
        .unwrap()
        .unwrap();
    // Mark the room #confidential.
    store
        .append(
            clock.now(),
            vec![
                EventPayload::TagCreated {
                    name: TagName::new("confidential"),
                    description: "a confidential room".to_owned(),
                },
                EventPayload::TagAppliedToMemory {
                    memory: context,
                    tag: TagName::new("confidential"),
                },
            ],
        )
        .unwrap();
    graph.materialize_from(&store).unwrap();

    // The agent records a topic in the confidential room. A topic write would normally default
    // public, and the agent teller is always present — but the confidential room forces it private,
    // so it cannot silently surface to whoever is around.
    let session = Session::new(conversation);
    session
        .execute(
            &mut Engine {
                store: &mut store,
                graph: &mut graph,
                clock: &clock,
            },
            &BlockContext {
                teller: Teller::Agent,
                authority: Authority::Platform,
                turn_id: TurnId::generate(),
            },
            r#"memory.create("topic/sensitive", "something said in confidence")"#,
        )
        .unwrap();

    let topic = graph.memory_by_name("topic/sensitive").unwrap().unwrap();
    let entries = graph.entries_local(topic.id).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].visibility, Visibility::PrivateToTeller);
}

#[test]
fn link_with_an_unregistered_relation_is_a_teachable_error() {
    let mut h = Harness::new();
    h.run(r#"memory.create("topic/a")"#);
    // No such relation is registered: the block fails with a teachable error and commits nothing.
    let outcome = h.run(r#"memory.get("topic/a"):link("bogus_rel", memory.get("topic/a"))"#);
    match outcome {
        BlockOutcome::Terminated(TerminalCause::Error(message)) => {
            assert!(
                message.contains("unknown relation"),
                "message was: {message}"
            );
        }
        other => panic!("expected a teachable error, got {other:?}"),
    }
}

#[test]
fn creating_a_duplicate_name_is_a_teachable_error() {
    let mut h = Harness::new();
    h.run(r#"memory.create("topic/plan", "first")"#);
    // Re-creating the same name is a teachable block error, not a fatal unique-constraint failure
    // that would poison the log.
    let outcome = h.run(r#"memory.create("topic/plan", "second")"#);
    match outcome {
        BlockOutcome::Terminated(TerminalCause::Error(message)) => {
            assert!(message.contains("already exists"), "message was: {message}");
        }
        other => panic!("expected a teachable error, got {other:?}"),
    }
    // The original memory is intact; the rejected create committed nothing.
    let plan = h.graph.memory_by_name("topic/plan").unwrap().unwrap();
    assert_eq!(h.graph.entries_local(plan.id).unwrap().len(), 1);
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
        memory.create("topic/ghost", "should not survive")
        block.abort("changed my mind")
        "#,
    );
    assert_eq!(
        outcome,
        BlockOutcome::Terminated(TerminalCause::Aborted("changed my mind".to_owned()))
    );
    // The buffered create was discarded.
    assert!(h.graph.memory_by_name("topic/ghost").unwrap().is_none());

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
        memory.create("topic/oops", "should not survive")
        error("boom")
        "#,
    );
    assert!(matches!(
        outcome,
        BlockOutcome::Terminated(TerminalCause::Error(_))
    ));
    assert!(h.graph.memory_by_name("topic/oops").unwrap().is_none());
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
