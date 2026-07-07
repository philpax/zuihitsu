use super::*;

#[tokio::test]
async fn block_commits_and_projects_with_read_your_writes() {
    let h = Harness::new();
    let outcome = h
        .run(
            r#"
        local dave = memory.create(PERSON_DAVE)
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
        .memory_by_name(Namespace::Person.with_name("dave"))
        .unwrap()
        .unwrap();
    assert_eq!(
        h.engine.graph.lock().entries_local(dave.id).unwrap().len(),
        2
    );
}

#[tokio::test]
async fn a_disputed_entry_reads_as_disputed() {
    // An entry under an unresolved belief arbitration renders with a `disputed` marker on read, so the
    // agent sees at a glance that a fact is contested and surfaces it rather than asserting it as
    // settled (spec §Lua API → reads render self-describingly).
    let h = Harness::new();
    h.run(
        r#"
        local ev = memory.create(EVENT_ALL_HANDS)
        ev:append("It is in the main auditorium.", { visibility = "public" })
        ev:append("It is on the rooftop terrace.", { visibility = "public" })
        return "ok"
        "#,
    )
    .await;

    let (memory, competing) = {
        let graph = h.engine.graph.lock();
        let ev = graph
            .memory_by_name(Namespace::Event.with_name("all-hands"))
            .unwrap()
            .unwrap();
        let competing: Vec<_> = graph
            .entries_local(ev.id)
            .unwrap()
            .into_iter()
            .map(|entry| entry.entry_id)
            .collect();
        (ev.id, competing)
    };

    // Inject the unresolved arbitration the synthesis pass would record, and project it.
    h.engine
        .store
        .lock()
        .as_mut()
        .append(
            h.clock.now(),
            vec![EventPayload::belief_arbitrated(
                memory,
                competing,
                ArbitrationResolution {
                    credited: Vec::new(),
                    statement: "one says auditorium, another rooftop".to_owned(),
                },
                None,
            )],
        )
        .unwrap();
    {
        let store = h.engine.store.lock();
        h.engine
            .graph
            .lock()
            .materialize_from(store.as_ref())
            .unwrap();
    }

    let outcome = h
        .run(r#"return memory.get(EVENT_ALL_HANDS):entries()"#)
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit");
    };
    assert_eq!(
        result.matches("[disputed").count(),
        2,
        "both competing entries should read as disputed, got: {result}"
    );
}

#[tokio::test]
async fn committed_memory_is_visible_to_a_later_block() {
    let h = Harness::new();
    h.run(r#"memory.create(TOPIC_SOURDOUGH, "A naturally leavened bread")"#)
        .await;
    let outcome = h
        .run(r#"return memory.get(TOPIC_SOURDOUGH):entries()"#)
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
        memory.create(TOPIC_GHOST, "should not survive")
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
            .memory_by_name(Namespace::Topic.with_name("ghost"))
            .unwrap()
            .is_none()
    );

    // A LuaExecuted recording the abort is still in the log (the agent saw the outcome).
    let aborted = h.events().into_iter().any(|e| {
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
        memory.create(TOPIC_OOPS, "should not survive")
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
            .memory_by_name(Namespace::Topic.with_name("oops"))
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn lua_executed_records_the_script_result_and_touched_set() {
    let h = Harness::new();
    h.run(r#"memory.create(PLACE_SYDNEY, "A harbour city") return "done""#)
        .await;

    let recorded = h
        .events()
        .into_iter()
        .find_map(|e| match e.payload {
            EventPayload::LuaExecuted {
                result, touched, ..
            } => Some((result, touched)),
            _ => None,
        })
        .expect("a LuaExecuted event");
    // The script result is recorded, now trailed by the committed-effects summary the agent also saw.
    let recorded_result = recorded.0.as_deref().expect("a recorded result");
    assert!(recorded_result.starts_with("done"));
    assert!(recorded_result.contains(&format!(
        "Committed: created {}",
        MemoryName::from(Namespace::Place.with_name("sydney")).as_str()
    )));
    assert_eq!(recorded.1.len(), 1); // touched the one created memory
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_block_waits_on_a_held_memory_lock_then_proceeds() {
    // Per-memory mutual exclusion (spec §Concurrency): a block touching a memory whose lock another
    // block holds waits until it is released. The lock is held externally here, standing in for a
    // concurrent block in another conversation.
    let h = Harness::new();
    h.run(r#"memory.create(TOPIC_SHARED, "one")"#).await;
    let id = h
        .engine
        .graph
        .lock()
        .memory_by_name(Namespace::Topic.with_name("shared"))
        .unwrap()
        .unwrap()
        .id;

    let guard = h.engine.memory_locks.acquire(id).await;

    // While the lock is held, a block touching that memory cannot finish (its own budget is far longer
    // than this window, so it is genuinely waiting on the lock, not self-aborting).
    let blocked = tokio::time::timeout(
        Duration::from_millis(200),
        h.run(r#"memory.get(TOPIC_SHARED):append("two")"#),
    )
    .await;
    assert!(blocked.is_err(), "the block should wait on the held lock");

    // Once the lock frees, a fresh attempt at the same block commits.
    drop(guard);
    let outcome = h.run(r#"memory.get(TOPIC_SHARED):append("two")"#).await;
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
                name: RelationName::SameAs,
                inverse: RelationName::SameAs,
                from_card: Cardinality::Many,
                to_card: Cardinality::Many,
                symmetric: true,
                reflexive: false,
                description: String::new(),
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
    h.run(r#"memory.create(PERSON_A)"#).await;
    h.run(r#"memory.create(PERSON_B_AT_DISCORD)"#).await;
    // A same_as merge needs operator authority (a platform turn may not merge).
    let operator = BlockContext {
        teller: Teller::Agent,
        authority: Authority::Operator,
        turn_id: TurnId::generate(),
        block_timeout: TEST_BLOCK_TIMEOUT,
        max_block_attempts: TEST_MAX_BLOCK_ATTEMPTS,
        present_set: Vec::new(),
        dry_run: false,
    };
    h.session
        .execute(
            &h.engine,
            &operator,
            &common::prepare_script(
                r#"memory.get(PERSON_A):link("same_as", memory.get(PERSON_B_AT_DISCORD))"#,
            ),
        )
        .await
        .unwrap();
    let sibling = h
        .engine
        .graph
        .lock()
        .memory_by_name(Namespace::Person.with_name("b@discord"))
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
        dry_run: false,
    };
    let blocked = h
        .session
        .execute(
            &h.engine,
            &starved,
            &common::prepare_script(r#"return memory.get(PERSON_A):entries()"#),
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
    let outcome = h.run(r#"return memory.get(PERSON_A):entries()"#).await;
    assert!(matches!(outcome, BlockOutcome::Committed { .. }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lock_starved_block_gives_up_after_its_attempts() {
    // Abort-and-retry (spec §Concurrency): a block that keeps timing out on a lock-wait, having made no
    // MCP call, is retried up to its bound and then gives up with a terminal error naming the count.
    let h = Harness::new();
    h.run(r#"memory.create(TOPIC_LOCKED, "x")"#).await;
    let id = h
        .engine
        .graph
        .lock()
        .memory_by_name(Namespace::Topic.with_name("locked"))
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
                dry_run: false,
            },
            &common::prepare_script(r#"memory.get(TOPIC_LOCKED):append("y")"#),
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
        local dave = memory.create(PERSON_DAVE)
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
    // The returned value, now trailed by the committed-effects summary (including the supersession).
    assert!(result.starts_with("live=1 history=2"));
    assert!(result.contains(&format!(
        "superseded an entry on {}",
        MemoryName::from(Namespace::Person.with_name("dave")).as_str()
    )));

    // Committed and projected: the live read shows only the correction; history shows both, with the
    // superseded entry's pointer stamped.
    let dave = h
        .engine
        .graph
        .lock()
        .memory_by_name(Namespace::Person.with_name("dave"))
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
async fn supersede_with_a_foreign_entry_is_a_teachable_error() {
    let h = Harness::new();
    // An entry from another memory is not a live entry of dave's class — a teachable misuse, not a
    // fatal error, and nothing commits.
    let outcome = h
        .run(
            r#"
        local dave = memory.create(PERSON_DAVE)
        local mine = dave:append("a real fact", { visibility = "public" })
        local erin = memory.create(PERSON_ERIN)
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
    let dave = h
        .engine
        .graph
        .lock()
        .memory_by_name(Namespace::Person.with_name("dave"))
        .unwrap();
    assert!(dave.is_none(), "the whole block was discarded");
}

#[tokio::test]
async fn the_block_vm_is_sandboxed_against_host_access() {
    // The Lua surface is an orchestration language over the projected API, never a host program: the
    // filesystem, the environment, the process, and arbitrary code on disk must be out of reach, so
    // MCP stays the only sanctioned outward path (spec §External I/O via MCP). A regression guard — a
    // stock `Lua::new()` would expose every one of these.
    let h = Harness::new();
    let outcome = h
        .run(
            r#"
        local exposed = {}
        for _, name in ipairs({ "os", "io", "package", "require", "dofile", "loadfile",
                                "load", "loadstring" }) do
            if _G[name] ~= nil then exposed[#exposed + 1] = name end
        end
        -- The pure orchestration libraries stay available.
        assert(type(string.format) == "function", "string library missing")
        assert(type(table.insert) == "function", "table library missing")
        assert(type(math.floor) == "function", "math library missing")
        return table.concat(exposed, ",")
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected the probe block to commit, got {outcome:?}");
    };
    assert_eq!(
        result.trim(),
        "",
        "these host globals must not be reachable from a block: {}",
        result.trim()
    );
}

#[tokio::test]
async fn a_write_block_reports_what_it_committed() {
    // A write block returns nil, which alone tells the agent nothing about whether its create and
    // append landed. The committed-effects summary stands in for that bare nil, so the agent sees its
    // writes took and does not re-issue them next turn (the soak-observed double-record). A read-only
    // query keeps its own rendered result, unchanged.
    let h = Harness::new();
    let outcome = h
        .run(
            r#"
        local plan = memory.create(TOPIC_Q3_PLAN)
        plan:append("Ship the database migration", { visibility = "public" })
        plan:append("Refresh the marketing site", { visibility = "public" })
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert!(
        result.contains(&format!(
            "Committed: created {}",
            MemoryName::from(Namespace::Topic.with_name("q3_plan")).as_str()
        )),
        "the write block should report its create: {result:?}"
    );
    assert!(
        result.contains(&format!(
            "appended 2 entries to {}",
            MemoryName::from(Namespace::Topic.with_name("q3_plan")).as_str()
        )),
        "the write block should report its appends: {result:?}"
    );

    // A read-only query in the same session reports its rendered value, with no commit summary.
    let outcome = h
        .run(r#"return #memory.get(TOPIC_Q3_PLAN):entries()"#)
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert_eq!(result, "2");
    assert!(
        !result.contains("Committed:"),
        "a read-only query should carry no commit summary: {result:?}"
    );
}
