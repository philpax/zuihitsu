//! Lua block-transactionality tests: a block's writes commit atomically and project to the graph;
//! reads see the block's own pending writes; scratchpad globals persist across the session's
//! blocks; and an abort or runtime error discards the buffer while recording the terminal cause
//! (spec §Lua API → block transactionality).

mod common;

use std::{sync::Arc, time::Duration};

use common::Harness;
use zuihitsu::{
    Authority, BEFORE_AFTER_EPSILON_MILLIS, BlockContext, BlockOutcome, Cardinality, CivilDate,
    Clock, ConversationLocator, Engine, Graph, ManualClock, MemoryId, MemoryName, MemoryStore,
    RelationName, Seq, Session, Store, TagName, Teller, TemporalRef, TerminalCause, TurnId,
    Visibility, event::EventPayload, resolve_or_mint_conversation,
};

/// A block-duration budget generous enough that these in-memory blocks never trip it.
const TEST_BLOCK_TIMEOUT: Duration = Duration::from_secs(30);
/// The per-block lock-wait retry bound for these tests.
const TEST_MAX_BLOCK_ATTEMPTS: u32 = 3;

#[tokio::test]
async fn block_commits_and_projects_with_read_your_writes() {
    let h = Harness::new();
    let outcome = h
        .run(
            r#"
        local dave = memory.create("person/dave")
        dave:append("Met at the climbing gym", { visibility = "public" })
        dave:append("Got a new job at Hooli", { visibility = "public" })
        return dave:entries()
        "#,
        )
        .await;

    // The block saw its own pending writes (read-your-writes), rendered back as the result.
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit");
    };
    assert!(result.contains("Met at the climbing gym"));
    assert!(result.contains("Got a new job at Hooli"));

    // And they committed and projected to the graph.
    let dave = h
        .engine
        .graph
        .lock()
        .memory_by_name("person/dave")
        .unwrap()
        .unwrap();
    assert_eq!(
        h.engine.graph.lock().entries_local(dave.id).unwrap().len(),
        2
    );
}

#[tokio::test]
async fn append_records_a_structured_occurred_at() {
    let h = Harness::new();
    let outcome = h
        .run(
            r#"
        local ev = memory.create("event/cleaning")
        ev:append("Scheduled cleaning", { visibility = "public", occurred_at = { day = "2026-06-03" } })
        return "ok"
        "#,
        )
        .await;
    assert!(matches!(outcome, BlockOutcome::Committed { .. }));

    // The tagged Lua table deserialized into a TemporalRef end to end, and the materializer
    // denormalized it to the day's noon in occurred_sort.
    let ev = h
        .engine
        .graph
        .lock()
        .memory_by_name("event/cleaning")
        .unwrap()
        .unwrap();
    let entries = h.engine.graph.lock().entries_local(ev.id).unwrap();
    assert_eq!(entries.len(), 1);
    let expected = TemporalRef::Day(CivilDate("2026-06-03".into()))
        .bounds(None, BEFORE_AFTER_EPSILON_MILLIS)
        .sort;
    assert_eq!(entries[0].occurred_sort, expected);
    assert!(expected.is_some());
}

#[tokio::test]
async fn calendar_queries_return_matching_memories() {
    let h = Harness::new();
    // Write in one block; calendar queries read the materialized graph (committed state), not the
    // block's own pending buffer, so they run in a later block.
    h.run(
        r#"
        local d = memory.create("event/cleaning")
        d:append("dentist", { visibility = "public", occurred_at = { day = "2026-06-03" } })
        local s = memory.create("event/standup")
        s:append("standup", { visibility = "public", occurred_at = { recurring = "FREQ=WEEKLY" } })
        "#,
    )
    .await;
    let outcome = h
        .run(r#"return #calendar.on("2026-06-03") .. "," .. #calendar.recurring()"#)
        .await;
    // calendar.on finds the day's concrete occurrence; calendar.recurring lists the recurring one.
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert_eq!(result, "1,1");
}

#[tokio::test]
async fn calendar_rejects_a_malformed_argument() {
    let h = Harness::new();
    let outcome = h.run(r#"return calendar.on("not-a-date")"#).await;
    match outcome {
        BlockOutcome::Terminated(TerminalCause::Error(message)) => {
            assert!(
                message.contains("calendar argument"),
                "message was: {message}"
            );
        }
        other => panic!("expected a teachable error, got {other:?}"),
    }
}

#[tokio::test]
async fn append_carries_teller_context_and_default_visibility() {
    let mut store = MemoryStore::new();
    let clock = ManualClock::new(common::time::EARLY);
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

    // The shared engine the block writes through, read back below via the same handle.
    let engine = Engine::new(Box::new(store), graph, Box::new(clock.clone()));
    async fn exec(session: &Session, engine: &Arc<Engine>, teller: MemoryId, script: &str) {
        session
            .execute(
                engine,
                &BlockContext {
                    teller: Teller::Participant(teller),
                    authority: Authority::Platform,
                    turn_id: TurnId::generate(),
                    block_timeout: TEST_BLOCK_TIMEOUT,
                    max_block_attempts: TEST_MAX_BLOCK_ATTEMPTS,
                    present_set: Vec::new(),
                },
                script,
            )
            .await
            .unwrap();
    }

    // Erin, in the room, relays something about Phil: attributed to her, told in this context, and
    // defaulted private to its teller because the subject (Phil) is not the teller.
    exec(
        &session,
        &engine,
        erin,
        r#"memory.get("person/phil"):append("is being managed out")"#,
    )
    .await;
    // `by_agent` records the agent's own observation about a person, which has no protective default
    // (the aside mechanism keys on a participant teller) — so it must classify the entry explicitly.
    exec(
        &session,
        &engine,
        erin,
        r#"memory.get("person/phil"):append("seems stressed", { by_agent = true, visibility = "public" })"#,
    )
    .await;
    exec(
        &session,
        &engine,
        erin,
        r#"memory.get("person/phil"):append("got promoted", { visibility = "public" })"#,
    )
    .await;

    let entries = engine.graph.lock().entries_local(phil).unwrap();
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
        &session,
        &engine,
        erin,
        r#"context.current():append("kept in confidence", { by_agent = true })"#,
    )
    .await;
    let context_entries = engine.graph.lock().entries_local(context).unwrap();
    assert_eq!(context_entries.len(), 1);
    assert_eq!(context_entries[0].text, "kept in confidence");
}

#[tokio::test]
async fn link_flags_a_memory_active_in_the_context_and_unlink_clears_it() {
    let mut store = MemoryStore::new();
    let clock = ManualClock::new(common::time::EARLY);
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

    let engine = Engine::new(Box::new(store), graph, Box::new(clock.clone()));
    let context_block = || BlockContext {
        teller: Teller::Agent,
        authority: Authority::Platform,
        turn_id: TurnId::generate(),
        block_timeout: TEST_BLOCK_TIMEOUT,
        max_block_attempts: TEST_MAX_BLOCK_ATTEMPTS,
        present_set: Vec::new(),
    };

    // The agent flags the thread active_in the current context.
    let outcome = session
        .execute(
            &engine,
            &context_block(),
            r#"memory.get("topic/roadmap"):link("active_in", context.current())"#,
        )
        .await
        .unwrap();
    assert!(matches!(outcome, BlockOutcome::Committed { .. }));
    // Read back through the has_active inverse: the context now carries the thread.
    let active = engine.graph.lock().outgoing(context, "has_active").unwrap();
    assert!(active.iter().any(|memory| memory.id == roadmap));

    // Unlinking clears it.
    session
        .execute(
            &engine,
            &context_block(),
            r#"memory.get("topic/roadmap"):unlink("active_in", context.current())"#,
        )
        .await
        .unwrap();
    assert!(
        engine
            .graph
            .lock()
            .outgoing(context, "has_active")
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn a_write_in_a_confidential_room_defaults_private() {
    let mut store = MemoryStore::new();
    let clock = ManualClock::new(common::time::EARLY);
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
    let engine = Engine::new(Box::new(store), graph, Box::new(clock.clone()));
    session
        .execute(
            &engine,
            &BlockContext {
                teller: Teller::Agent,
                authority: Authority::Platform,
                turn_id: TurnId::generate(),
                block_timeout: TEST_BLOCK_TIMEOUT,
                max_block_attempts: TEST_MAX_BLOCK_ATTEMPTS,
                present_set: Vec::new(),
            },
            r#"memory.create("topic/sensitive", "something said in confidence")"#,
        )
        .await
        .unwrap();

    let topic = engine
        .graph
        .lock()
        .memory_by_name("topic/sensitive")
        .unwrap()
        .unwrap();
    let entries = engine.graph.lock().entries_local(topic.id).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].visibility, Visibility::PrivateToTeller);
}

#[tokio::test]
async fn link_with_an_unregistered_relation_is_a_teachable_error() {
    let h = Harness::new();
    h.run(r#"memory.create("topic/a")"#).await;
    // No such relation is registered: the block fails with a teachable error and commits nothing.
    let outcome = h
        .run(r#"memory.get("topic/a"):link("bogus_rel", memory.get("topic/a"))"#)
        .await;
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

#[tokio::test]
async fn creating_a_duplicate_name_is_a_teachable_error() {
    let h = Harness::new();
    h.run(r#"memory.create("topic/plan", "first")"#).await;
    // Re-creating the same name is a teachable block error, not a fatal unique-constraint failure
    // that would poison the log.
    let outcome = h.run(r#"memory.create("topic/plan", "second")"#).await;
    match outcome {
        BlockOutcome::Terminated(TerminalCause::Error(message)) => {
            assert!(message.contains("already exists"), "message was: {message}");
        }
        other => panic!("expected a teachable error, got {other:?}"),
    }
    // The original memory is intact; the rejected create committed nothing.
    let plan = h
        .engine
        .graph
        .lock()
        .memory_by_name("topic/plan")
        .unwrap()
        .unwrap();
    assert_eq!(
        h.engine.graph.lock().entries_local(plan.id).unwrap().len(),
        1
    );
}

#[tokio::test]
async fn committed_memory_is_visible_to_a_later_block() {
    let h = Harness::new();
    h.run(r#"memory.create("topic/sourdough", "A naturally leavened bread")"#)
        .await;
    let outcome = h
        .run(r#"return memory.get("topic/sourdough"):entries()"#)
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit");
    };
    assert!(result.contains("naturally leavened"));
}

#[tokio::test]
async fn scratchpad_globals_persist_across_blocks() {
    let h = Harness::new();
    h.run("scratch = 41").await;
    let outcome = h.run("return scratch + 1").await;
    assert_eq!(
        outcome,
        BlockOutcome::Committed {
            result: "42".to_owned()
        }
    );
}

#[tokio::test]
async fn abort_discards_the_buffer() {
    let h = Harness::new();
    let outcome = h
        .run(
            r#"
        memory.create("topic/ghost", "should not survive")
        block.abort("changed my mind")
        "#,
        )
        .await;
    assert_eq!(
        outcome,
        BlockOutcome::Terminated(TerminalCause::Aborted("changed my mind".to_owned()))
    );
    // The buffered create was discarded.
    assert!(
        h.engine
            .graph
            .lock()
            .memory_by_name("topic/ghost")
            .unwrap()
            .is_none()
    );

    // A LuaExecuted recording the abort is still in the log (the agent saw the outcome).
    let aborted = h
        .engine
        .store
        .lock()
        .read_from(Seq::ZERO)
        .unwrap()
        .into_iter()
        .any(|e| {
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

#[tokio::test]
async fn runtime_error_discards_the_buffer_and_records_the_cause() {
    let h = Harness::new();
    let outcome = h
        .run(
            r#"
        memory.create("topic/oops", "should not survive")
        error("boom")
        "#,
        )
        .await;
    assert!(matches!(
        outcome,
        BlockOutcome::Terminated(TerminalCause::Error(_))
    ));
    assert!(
        h.engine
            .graph
            .lock()
            .memory_by_name("topic/oops")
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn lua_executed_records_the_script_result_and_touched_set() {
    let h = Harness::new();
    h.run(r#"memory.create("place/sydney", "A harbour city") return "done""#)
        .await;

    let recorded = h
        .engine
        .store
        .lock()
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_block_waits_on_a_held_memory_lock_then_proceeds() {
    // Per-memory mutual exclusion (spec §Concurrency): a block touching a memory whose lock another
    // block holds waits until it is released. The lock is held externally here, standing in for a
    // concurrent block in another conversation.
    let h = Harness::new();
    h.run(r#"memory.create("topic/shared", "one")"#).await;
    let id = h
        .engine
        .graph
        .lock()
        .memory_by_name("topic/shared")
        .unwrap()
        .unwrap()
        .id;

    let guard = h.engine.memory_locks.acquire(id).await;

    // While the lock is held, a block touching that memory cannot finish (its own budget is far longer
    // than this window, so it is genuinely waiting on the lock, not self-aborting).
    let blocked = tokio::time::timeout(
        Duration::from_millis(200),
        h.run(r#"memory.get("topic/shared"):append("two")"#),
    )
    .await;
    assert!(blocked.is_err(), "the block should wait on the held lock");

    // Once the lock frees, a fresh attempt at the same block commits.
    drop(guard);
    let outcome = h.run(r#"memory.get("topic/shared"):append("two")"#).await;
    assert!(matches!(outcome, BlockOutcome::Committed { .. }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_traversing_read_locks_the_whole_class() {
    // Class-wide locking (spec §Concurrency): a traversing read (mem:entries) locks the full same_as
    // class, so it waits on a sibling stub's lock even though it queried a different member.
    let h = Harness::new();
    // The Harness skips genesis, so register the same_as relation the merge needs.
    h.engine
        .store
        .lock()
        .append(
            h.clock.now(),
            vec![EventPayload::LinkTypeRegistered {
                name: RelationName::new("same_as"),
                inverse: RelationName::new("same_as"),
                from_card: Cardinality::Many,
                to_card: Cardinality::Many,
                symmetric: true,
                reflexive: false,
            }],
        )
        .unwrap();
    h.engine
        .graph
        .lock()
        .materialize_from(h.engine.store.lock().as_ref())
        .unwrap();
    // Create the two stubs (no content — an agent-authored note about a person would need explicit
    // visibility, and the class lock does not depend on content).
    h.run(r#"memory.create("person/a")"#).await;
    h.run(r#"memory.create("person/b@discord")"#).await;
    // A same_as merge needs operator authority (a platform turn may not merge).
    let operator = BlockContext {
        teller: Teller::Agent,
        authority: Authority::Operator,
        turn_id: TurnId::generate(),
        block_timeout: TEST_BLOCK_TIMEOUT,
        max_block_attempts: TEST_MAX_BLOCK_ATTEMPTS,
        present_set: Vec::new(),
    };
    h.session
        .execute(
            &h.engine,
            &operator,
            r#"memory.get("person/a"):link("same_as", memory.get("person/b@discord"))"#,
        )
        .await
        .unwrap();
    let sibling = h
        .engine
        .graph
        .lock()
        .memory_by_name("person/b@discord")
        .unwrap()
        .unwrap()
        .id;

    // Hold the sibling's lock. A traversing read of the *other* member locks the whole class, so it
    // waits on the sibling and — with a short budget and a single attempt — gives up. Driving it
    // through `execute`'s own timeout (not an outer cancellation) means the block releases its locks on
    // the way out.
    let guard = h.engine.memory_locks.acquire(sibling).await;
    let starved = BlockContext {
        teller: Teller::Agent,
        authority: Authority::Platform,
        turn_id: TurnId::generate(),
        block_timeout: Duration::from_millis(60),
        max_block_attempts: 1,
        present_set: Vec::new(),
    };
    let blocked = h
        .session
        .execute(
            &h.engine,
            &starved,
            r#"return memory.get("person/a"):entries()"#,
        )
        .await
        .unwrap();
    assert!(
        matches!(blocked, BlockOutcome::Terminated(TerminalCause::Error(_))),
        "the traversing read should have waited on the sibling's class lock and timed out, got {blocked:?}"
    );

    // With the sibling free, the same traversing read commits — confirming the sibling's lock was what
    // it waited on.
    drop(guard);
    let outcome = h.run(r#"return memory.get("person/a"):entries()"#).await;
    assert!(matches!(outcome, BlockOutcome::Committed { .. }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lock_starved_block_gives_up_after_its_attempts() {
    // Abort-and-retry (spec §Concurrency): a block that keeps timing out on a lock-wait, having made no
    // MCP call, is retried up to its bound and then gives up with a terminal error naming the count.
    let h = Harness::new();
    h.run(r#"memory.create("topic/locked", "x")"#).await;
    let id = h
        .engine
        .graph
        .lock()
        .memory_by_name("topic/locked")
        .unwrap()
        .unwrap()
        .id;
    // Held for the whole test, so every attempt times out.
    let _guard = h.engine.memory_locks.acquire(id).await;

    let outcome = h
        .session
        .execute(
            &h.engine,
            &BlockContext {
                teller: Teller::Agent,
                authority: Authority::Platform,
                turn_id: TurnId::generate(),
                block_timeout: Duration::from_millis(40),
                max_block_attempts: 2,
                present_set: Vec::new(),
            },
            r#"memory.get("topic/locked"):append("y")"#,
        )
        .await
        .unwrap();
    match outcome {
        BlockOutcome::Terminated(TerminalCause::Error(message)) => {
            assert!(message.contains("2 attempts"), "message was {message:?}");
        }
        other => panic!("expected a give-up terminal, got {other:?}"),
    }
}

#[tokio::test]
async fn supersede_drops_an_entry_from_live_reads_but_keeps_it_in_history() {
    let h = Harness::new();
    // In one block: record a fact, append the correction, supersede the old with the new. The block's
    // own live read reflects the correction (read-your-writes); history keeps both.
    let outcome = h
        .run(
            r#"
        local dave = memory.create("person/dave")
        local old = dave:append("Dave works at Hooli", { visibility = "public" })
        local new = dave:append("Dave works at Pied Piper", { visibility = "public" })
        dave:supersede(old, new)
        return "live=" .. #dave:entries() .. " history=" .. #dave:history()
        "#,
        )
        .await;

    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert_eq!(result, "live=1 history=2");

    // Committed and projected: the live read shows only the correction; history shows both, with the
    // superseded entry's pointer stamped.
    let dave = h
        .engine
        .graph
        .lock()
        .memory_by_name("person/dave")
        .unwrap()
        .unwrap();
    let live: Vec<String> = h
        .engine
        .graph
        .lock()
        .entries_local(dave.id)
        .unwrap()
        .into_iter()
        .map(|e| e.text)
        .collect();
    assert_eq!(live, ["Dave works at Pied Piper"]);
    let history = h.engine.graph.lock().class_history(dave.id).unwrap();
    assert_eq!(history.len(), 2);
    let superseded = history
        .iter()
        .find(|e| e.text == "Dave works at Hooli")
        .unwrap();
    assert!(superseded.superseded_by.is_some());
}

#[tokio::test]
async fn entries_render_as_their_text_and_concatenate() {
    let h = Harness::new();
    // An entry handle reads as its text: returned in a list (rendered for the model) and via `..`.
    let outcome = h
        .run(
            r#"
        local dave = memory.create("person/dave")
        dave:append("climbs on Tuesdays", { visibility = "public" })
        local entries = dave:entries()
        return "first: " .. entries[1]
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert_eq!(result, "first: climbs on Tuesdays");
}

#[tokio::test]
async fn an_undecorated_table_renders_as_its_structure_not_an_opaque_token() {
    let h = Harness::new();
    // A plain map table the agent builds and returns has no `__tostring`, so before it rendered as
    // the information-free `<table>`. It now pretty-prints through the vendored `inspect`, so the
    // model reads back the fields it returned.
    let outcome = h
        .run(
            r#"
        return { name = "person/dave", role = "climber", visits = 3 }
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert_ne!(result, "<table>", "an undecorated table must not render opaquely");
    assert!(result.contains("name"), "structure should be visible: {result}");
    assert!(result.contains("person/dave"), "values should be visible: {result}");
    assert!(result.contains("visits"), "every key should be visible: {result}");
}

#[tokio::test]
async fn supersede_with_a_foreign_entry_is_a_teachable_error() {
    let h = Harness::new();
    // An entry from another memory is not a live entry of dave's class — a teachable misuse, not a
    // fatal error, and nothing commits.
    let outcome = h
        .run(
            r#"
        local dave = memory.create("person/dave")
        local mine = dave:append("a real fact", { visibility = "public" })
        local erin = memory.create("person/erin")
        local theirs = erin:append("erin's fact", { visibility = "public" })
        dave:supersede(theirs, mine)
        "#,
        )
        .await;
    match outcome {
        BlockOutcome::Terminated(TerminalCause::Error(message)) => {
            assert!(message.contains("no live entry"), "message was: {message}");
        }
        other => panic!("expected a teachable error, got {other:?}"),
    }
    // The rejected supersede committed nothing: both facts are still live.
    let dave = h.engine.graph.lock().memory_by_name("person/dave").unwrap();
    assert!(dave.is_none(), "the whole block was discarded");
}

#[tokio::test]
async fn a_created_tag_can_be_applied_and_listed() {
    let h = Harness::new();
    // Create a tag and apply it to a memory in one block (read-your-writes recognizes the pending
    // creation), which commits.
    let seeded = h
        .run(
            r#"
        tags.create("hobbies", "Recreational activities and interests")
        local dave = memory.create("person/dave")
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
        .memory_by_name("person/dave")
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
        local dave = memory.create("person/dave")
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
            .memory_by_name("person/dave")
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn creating_a_duplicate_tag_is_a_teachable_error() {
    let h = Harness::new();
    // Create a tag, which commits.
    let seeded = h
        .run(r#"tags.create("hobbies", "first purpose")"#)
        .await;
    assert!(matches!(seeded, BlockOutcome::Committed { .. }));

    // Re-creating it is a teachable error — creation forces a fresh purpose, so a collision points at
    // tags.describe to change one instead.
    let outcome = h
        .run(r#"tags.create("hobbies", "second purpose")"#)
        .await;
    match outcome {
        BlockOutcome::Terminated(TerminalCause::Error(message)) => {
            assert!(message.contains("already exists"), "message was: {message}");
            assert!(message.contains("tags.describe"), "message was: {message}");
        }
        other => panic!("expected a teachable error, got {other:?}"),
    }
}

#[tokio::test]
async fn memory_search_recalls_an_indexed_entry() {
    let h = Harness::with_retrieval();
    // Write a public fact, then embed it into the vector index.
    let seeded = h
        .run(
            r#"
        local dave = memory.create("person/dave")
        dave:append("An avid rock climber", { by_agent = true, visibility = "public" })
        return "ok"
        "#,
        )
        .await;
    assert!(matches!(seeded, BlockOutcome::Committed { .. }));
    h.index().await;

    // A search for the same text recalls Dave (the deterministic fake embedder matches it exactly);
    // each result is a { name, description, score, marker? } table.
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
    assert_eq!(result, "person/dave");

    // Returning the result list renders as readable lines (each result's __tostring), not "<table>",
    // so the agent can read its own search back.
    let rendered = h
        .run(r#"return memory.search("An avid rock climber")"#)
        .await;
    let BlockOutcome::Committed { result } = rendered else {
        panic!("expected commit, got {rendered:?}");
    };
    assert!(result.contains("person/dave"), "rendered: {result:?}");
    assert!(!result.contains("<table>"), "rendered: {result:?}");
}

#[tokio::test]
async fn memory_search_without_an_embedder_is_a_teachable_error() {
    // A graph-only harness has no retrieval, so search reports itself unavailable rather than failing
    // obscurely — and commits nothing.
    let h = Harness::new();
    let outcome = h.run(r#"return memory.search("anything")"#).await;
    match outcome {
        BlockOutcome::Terminated(TerminalCause::Error(message)) => {
            assert!(message.contains("unavailable"), "message was: {message}");
        }
        other => panic!("expected a teachable error, got {other:?}"),
    }
}
