//! Lua block-transactionality tests: a block's writes commit atomically and project to the graph;
//! reads see the block's own pending writes; scratchpad globals persist across the session's
//! blocks; and an abort or runtime error discards the buffer while recording the terminal cause
//! (spec §Lua API → block transactionality).

mod common;

use std::{sync::Arc, time::Duration};

use common::Harness;
use zuihitsu::{
    Authority, BEFORE_AFTER_EPSILON_MILLIS, BlockContext, BlockOutcome, Cardinality, CivilDate,
    Clock, Completion, ConversationLocator, Engine, Graph, InstanceFeatures, ManualClock, MemoryId,
    MemoryName, MemoryStore, Namespace, PromptTemplateName, RelationName, ScriptedModel, Session,
    Store, TagName, Teller, TemporalRef, TerminalCause, TurnId, Visibility,
    event::{ArbitrationResolution, EventPayload, EventSource},
    resolve_or_mint_conversation,
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
async fn a_dated_entry_reads_with_its_date() {
    // A dated fact renders its occurrence inline on read, so the agent sees *when* it happens without
    // inspecting a structured field or searching for a date that lives outside the entry text (spec
    // §Lua API → reads render self-describingly).
    let h = Harness::new();
    let outcome = h
        .run(
            r#"
        local ev = memory.create(EVENT_PRODUCT_LAUNCH)
        ev:append("Penciled in by Phil", { visibility = "public", occurred_at = { day = "2027-03-15" } })
        return ev:entries()
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit");
    };
    assert!(
        result.contains("[2027-03-15") && result.contains("Penciled in by Phil"),
        "the dated entry should render its date inline, got: {result}"
    );
}

#[tokio::test]
async fn an_entry_occurred_at_round_trips_for_supersede() {
    // A read's occurred_at is the same tagged table append takes, so a script can match an entry by
    // entry.occurred_at.day and supersede it — the update path that silently no-opped when occurred_at
    // read back as a rendered string (entry.occurred_at.day was then nil).
    let h = Harness::new();
    let outcome = h
        .run(
            r#"
        local ev = memory.create(EVENT_LAUNCH)
        ev:append("Launch", { occurred_at = { day = "2027-03-15" }, visibility = "public" })
        local old
        for _, e in ipairs(ev:entries()) do
            if e.occurred_at and e.occurred_at.day == "2027-03-15" then old = e end
        end
        local new = ev:append("Launch", { occurred_at = { day = "2027-03-22" }, visibility = "public" })
        ev:supersede(old, new)
        return ev:entries()
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit");
    };
    assert!(
        result.contains("[2027-03-22") && !result.contains("[2027-03-15"),
        "matching by occurred_at.day should have superseded the 15th with the 22nd, got: {result}"
    );
}

#[tokio::test]
async fn revise_appends_and_supersedes_a_fact_in_one_call() {
    // m:revise(old, new_text, opts) is append-then-supersede in one call — the find-and-supersede flow
    // without the two-step (#45). The 15th entry is replaced by the 22nd in a single call; the live
    // read shows only the new value, and history retains both.
    let h = Harness::new();
    let outcome = h
        .run(
            r#"
        local ev = memory.create(EVENT_LAUNCH)
        ev:append("Launch", { occurred_at = { day = "2027-03-15" }, visibility = "public" })
        local old
        for _, e in ipairs(ev:entries()) do
            if e.occurred_at and e.occurred_at.day == "2027-03-15" then old = e end
        end
        ev:revise(old, "Launch", { occurred_at = { day = "2027-03-22" }, visibility = "public" })
        return ev:entries()
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit");
    };
    assert!(
        result.contains("[2027-03-22") && !result.contains("[2027-03-15"),
        "revise should have superseded the 15th with the 22nd in one call, got: {result}"
    );
    // The superseded value survives in history (it dropped only from the live read).
    let BlockOutcome::Committed { result: hist } =
        h.run(r#"return memory.get(EVENT_LAUNCH):history()"#).await
    else {
        panic!("expected commit");
    };
    assert!(
        hist.contains("[2027-03-15") && hist.contains("[2027-03-22"),
        "history should retain both the old and new values, got: {hist}"
    );
}

#[tokio::test]
async fn calendar_computes_dates_for_occurred_at() {
    // The agent names a relative date and the runtime computes it, so the recorded occurrence is
    // correct without the model doing date arithmetic in its head (spec §Calendar → date arithmetic is
    // the runtime's job). The Harness clock is anchored at Monday 2026-06-08.
    let h = Harness::new();
    let outcome = h
        .run(
            r#"
        local ev = memory.create(EVENT_BOARD_UPDATE)
        ev:append("Send the board update", { occurred_at = calendar.next("friday"), visibility = "public" })
        return ev:entries()
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit");
    };
    // "this Friday" from Monday 2026-06-08 is 2026-06-12 — computed by the runtime, rendered on read.
    assert!(
        result.contains("[2026-06-12"),
        "the computed Friday should land as the occurrence, got: {result}"
    );
}

#[tokio::test]
async fn memory_create_accepts_occurred_at_in_its_options_table() {
    // `memory.create` previously only accepted `(name, content)` and silently ignored a third options
    // table, so reminders created in one call lost their `occurred_at` and never fired. The options table
    // now flows through to the first entry exactly like `mem:append`.
    let h = Harness::new(); // clock at Monday 2026-06-08.
    let outcome = h
        .run(
            r#"
        local ev = memory.create(EVENT_BOARD_UPDATE, "Send the board update", {
            occurred_at = calendar.next("friday"),
            visibility = "public"
        })
        return ev:entries()
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit");
    };
    assert!(
        result.contains("[2026-06-12"),
        "the created entry should carry the computed Friday as its occurrence, got: {result}"
    );
}

#[tokio::test]
async fn calendar_upcoming_surfaces_a_recurring_instance() {
    // A recurring memory whose next virtual instance falls in the window surfaces in calendar.upcoming,
    // so the agent's own calendar query sees a standup it set for "every Monday" rather than coming up
    // empty (spec §Calendar). Reproduces the recurring_reminder miss.
    let h = Harness::new(); // clock at Monday 2026-06-08
    h.run(
        r#"
        local e = memory.create(EVENT_STANDUP, "Team standup")
        e:append("Recurring every Monday", { occurred_at = { recurring = "FREQ=WEEKLY;BYDAY=MO" }, visibility = "public" })
        return "ok"
        "#,
    )
    .await;
    // Advance past the first instance into the next week, as the scenario does before the fresh turn.
    h.clock.advance_millis(8 * 86_400_000 + 34_000);
    let outcome = h
        .run(
            r#"
        local names = {}
        for _, m in ipairs(calendar.upcoming({ within = "7 days" })) do
            table.insert(names, m.name)
        end
        return "[" .. table.concat(names, ",") .. "]"
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit");
    };
    // The recurring instance surfaces, and the handle reads its name (the bug: m.name was nil).
    assert!(
        result.contains(MemoryName::from(Namespace::Event.with_name("standup")).as_str()),
        "the recurring standup should surface in upcoming and read its name, got: {result}"
    );
}

#[tokio::test]
async fn calendar_date_objects_carry_arithmetic() {
    // Date objects render as their ISO day and carry calendar-correct arithmetic (month clamping
    // included), so the agent composes dates from operations rather than computing them.
    let h = Harness::new();
    let outcome = h
        .run(
            r#"
        return tostring(calendar.today())
            .. " | " .. tostring(calendar.in_weeks(2))
            .. " | " .. tostring(calendar.date("2026-01-31"):add_months(1))
            .. " | " .. calendar.today():weekday()
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit");
    };
    assert_eq!(result, "2026-06-08 | 2026-06-22 | 2026-02-28 | Monday");
}

#[tokio::test]
async fn append_records_a_structured_occurred_at() {
    let h = Harness::new();
    let outcome = h
        .run(
            r#"
        local ev = memory.create(EVENT_CLEANING)
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
        .memory_by_name(Namespace::Event.with_name("cleaning"))
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
        local d = memory.create(EVENT_CLEANING)
        d:append("dentist", { visibility = "public", occurred_at = { day = "2026-06-03" } })
        local s = memory.create(EVENT_STANDUP)
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
async fn calendar_upcoming_includes_recurring_instances() {
    let h = Harness::new();
    // A weekly recurring event, recorded in one block (committed) so a later block's calendar query
    // reads it from the materialized graph.
    h.run(
        r#"
        local s = memory.create(EVENT_STANDUP)
        s:append("Weekly standup", { visibility = "public", occurred_at = { recurring = "FREQ=WEEKLY" } })
        "#,
    )
    .await;

    // Its next instance falls inside a two-week window, so upcoming surfaces it — recurring instances
    // now interleave with concrete occurrences rather than being invisible to the calendar.
    let outcome = h
        .run(
            r#"
        local target = memory.get(EVENT_STANDUP)
        for _, m in ipairs(calendar.upcoming({ within = "14 days" })) do
            if m.id == target.id then return "found" end
        end
        return "missing"
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert_eq!(result, "found");
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
                EventPayload::memory_created(phil, Namespace::Person.with_name("phil")),
                EventPayload::memory_created(erin, Namespace::Person.with_name("erin")),
            ],
        )
        .unwrap();
    graph.materialize_from(&store).unwrap();
    let context = graph
        .context_for_conversation(conversation)
        .unwrap()
        .unwrap();
    let session = Session::new(conversation, InstanceFeatures::default());

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
                    dry_run: false,
                },
                &common::prepare_script(script),
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
        r#"memory.get(PERSON_PHIL):append("is being managed out")"#,
    )
    .await;
    // `by_agent` records the agent's own observation about a person, which has no protective default
    // (the aside mechanism keys on a participant teller) — so it must classify the entry explicitly.
    exec(
        &session,
        &engine,
        erin,
        r#"memory.get(PERSON_PHIL):append("seems stressed", { by_agent = true, visibility = "public" })"#,
    )
    .await;
    exec(
        &session,
        &engine,
        erin,
        r#"memory.get(PERSON_PHIL):append("got promoted", { visibility = "public" })"#,
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
async fn link_flags_a_memory_session_carryover_the_context_and_unlink_clears_it() {
    let mut store = MemoryStore::new();
    let clock = ManualClock::new(common::time::EARLY);
    let mut graph = Graph::open_in_memory().unwrap();

    // A room (with its context memory), the _session_carryover relation, and a thread memory.
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
                    name: RelationName::SessionCarryover,
                    inverse: RelationName::SessionCarries,
                    from_card: Cardinality::Many,
                    to_card: Cardinality::Many,
                    symmetric: false,
                    reflexive: false,
                    description: String::new(),
                },
                EventPayload::memory_created(roadmap, Namespace::Topic.with_name("roadmap")),
            ],
        )
        .unwrap();
    graph.materialize_from(&store).unwrap();
    let context = graph
        .context_for_conversation(conversation)
        .unwrap()
        .unwrap();
    let session = Session::new(conversation, InstanceFeatures::default());

    let engine = Engine::new(Box::new(store), graph, Box::new(clock.clone()));
    let context_block = || BlockContext {
        teller: Teller::Agent,
        authority: Authority::Platform,
        turn_id: TurnId::generate(),
        block_timeout: TEST_BLOCK_TIMEOUT,
        max_block_attempts: TEST_MAX_BLOCK_ATTEMPTS,
        present_set: Vec::new(),
        dry_run: false,
    };

    // The agent flags the thread _session_carryover the current context.
    let outcome = session
        .execute(
            &engine,
            &context_block(),
            &common::prepare_script(
                r#"memory.get(TOPIC_ROADMAP):link("_session_carryover", context.current())"#,
            ),
        )
        .await
        .unwrap();
    assert!(matches!(outcome, BlockOutcome::Committed { .. }));
    // Read back through the _session_carries inverse: the context now carries the thread.
    let active = engine
        .graph
        .lock()
        .outgoing(context, RelationName::SessionCarries.as_str())
        .unwrap();
    assert!(active.iter().any(|memory| memory.id == roadmap));

    // Unlinking clears it.
    session
        .execute(
            &engine,
            &context_block(),
            &common::prepare_script(
                r#"memory.get(TOPIC_ROADMAP):unlink("_session_carryover", context.current())"#,
            ),
        )
        .await
        .unwrap();
    assert!(
        engine
            .graph
            .lock()
            .outgoing(context, RelationName::SessionCarries.as_str())
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
                EventPayload::tag_created(
                    TagName::new("confidential"),
                    "a confidential room".to_owned(),
                ),
                EventPayload::tag_applied_to_memory(context, TagName::new("confidential")),
            ],
        )
        .unwrap();
    graph.materialize_from(&store).unwrap();

    // The agent records a topic in the confidential room. A topic write would normally default
    // public, and the agent teller is always present — but the confidential room forces it private,
    // so it cannot silently surface to whoever is around.
    let session = Session::new(conversation, InstanceFeatures::default());
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
                dry_run: false,
            },
            &common::prepare_script(
                r#"memory.create(TOPIC_SENSITIVE, "something said in confidence")"#,
            ),
        )
        .await
        .unwrap();

    let topic = engine
        .graph
        .lock()
        .memory_by_name(Namespace::Topic.with_name("sensitive"))
        .unwrap()
        .unwrap();
    let entries = engine.graph.lock().entries_local(topic.id).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].visibility, Visibility::PrivateToTeller);
}

#[tokio::test]
async fn link_with_an_unregistered_relation_is_a_teachable_error() {
    let h = Harness::new();
    h.run(r#"memory.create(TOPIC_A)"#).await;
    // No such relation is registered: the block fails with a teachable error and commits nothing.
    let outcome = h
        .run(r#"memory.get(TOPIC_A):link("bogus_rel", memory.get(TOPIC_A))"#)
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
async fn link_resolves_a_name_string_target() {
    // A name string in place of a handle is looked up, not rejected with a type error that would roll
    // the whole block back — the cascade that silently dropped a co-located private write (#43). This
    // block links via a string *and* appends a confidence in one go; both must survive together.
    let h = Harness::new();
    // The Harness skips genesis, so register the `knows` relation the link instantiates.
    h.engine
        .store
        .lock()
        .append(
            h.clock.now(),
            vec![EventPayload::LinkTypeRegistered {
                name: RelationName::Knows,
                inverse: RelationName::Knows,
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
    h.run(r#"memory.create(PERSON_DAVE)"#).await;
    h.run(r#"memory.create(PERSON_ERIN)"#).await;

    // PERSON_ERIN substitutes to a bare name string, not a handle, so this exercises the string-target
    // path; the private append in the same block proves it does not error-and-roll-back.
    let outcome = h
        .run(
            r#"local dave = memory.get(PERSON_DAVE)
               dave:link("knows", PERSON_ERIN)
               dave:append("a quiet aside", { visibility = "private" })"#,
        )
        .await;
    assert!(
        matches!(outcome, BlockOutcome::Committed { .. }),
        "a string-target link must commit (with its co-located write), got {outcome:?}"
    );

    // The string target resolved to a real edge — an outgoing `knows` link now exists.
    let BlockOutcome::Committed { result } = h
        .run(r#"return memory.get(PERSON_DAVE):outgoing("knows")"#)
        .await
    else {
        panic!("expected a committed read");
    };
    assert!(
        !result.trim().is_empty(),
        "a knows edge should exist, got empty: {result:?}"
    );
}

#[tokio::test]
async fn creating_a_duplicate_name_is_a_teachable_error() {
    let h = Harness::new();
    h.run(r#"memory.create(TOPIC_PLAN, "first")"#).await;
    // Re-creating the same name is a teachable block error, not a fatal unique-constraint failure
    // that would poison the log.
    let outcome = h.run(r#"memory.create(TOPIC_PLAN, "second")"#).await;
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
        .memory_by_name(Namespace::Topic.with_name("plan"))
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

#[tokio::test]
async fn link_readers_traverse_the_merged_identity() {
    // The link readers (spec §Lua API → link readers) auto-traverse the same_as class: an edge on one
    // stub surfaces when read through any member, oriented against the identity, with the same_as
    // plumbing itself excluded.
    let h = Harness::new();
    // The Harness skips genesis, so register the relations the test links under.
    h.engine
        .store
        .lock()
        .append(
            h.clock.now(),
            vec![
                EventPayload::LinkTypeRegistered {
                    name: RelationName::SameAs,
                    inverse: RelationName::SameAs,
                    from_card: Cardinality::Many,
                    to_card: Cardinality::Many,
                    symmetric: true,
                    reflexive: false,
                    description: String::new(),
                },
                EventPayload::LinkTypeRegistered {
                    name: RelationName::new("mentor_of"),
                    inverse: RelationName::new("mentored_by"),
                    from_card: Cardinality::Many,
                    to_card: Cardinality::Many,
                    symmetric: false,
                    reflexive: false,
                    description: String::new(),
                },
                EventPayload::LinkTypeRegistered {
                    name: RelationName::new("works_at"),
                    inverse: RelationName::new("employs"),
                    from_card: Cardinality::Many,
                    to_card: Cardinality::One,
                    symmetric: false,
                    reflexive: false,
                    description: String::new(),
                },
            ],
        )
        .unwrap();
    h.engine
        .graph
        .lock()
        .materialize_from(h.engine.store.lock().as_ref())
        .unwrap();

    // A two-stub Dave identity, plus the people and the company it links to.
    for name in [
        MemoryName::from(Namespace::Person.with_name("dave")).as_str(),
        MemoryName::from(Namespace::Person.with_name("dave@discord")).as_str(),
        MemoryName::from(Namespace::Person.with_name("erin")).as_str(),
        MemoryName::from(Namespace::Person.with_name("frank")).as_str(),
        "company/hooli",
    ] {
        h.run(&format!("memory.create({name:?})")).await;
    }

    // Merge the two Dave stubs — operator-only.
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
                r#"memory.get(PERSON_DAVE):link("same_as", memory.get(PERSON_DAVE_AT_DISCORD))"#,
            ),
        )
        .await
        .unwrap();

    // Links spread across the two stubs: one mentors Erin, Frank mentors the other, and the other
    // works at Hooli — so a class-blind read of the primary stub would miss two of the three.
    h.run(r#"memory.get(PERSON_DAVE):link("mentor_of", memory.get(PERSON_ERIN))"#)
        .await;
    h.run(r#"memory.get(PERSON_FRANK):link("mentor_of", memory.get(PERSON_DAVE_AT_DISCORD))"#)
        .await;
    h.run(r#"memory.get(PERSON_DAVE_AT_DISCORD):link("works_at", memory.get("company/hooli"))"#)
        .await;

    // outgoing: who Dave mentors — Erin, reached through the merged identity though queried via the
    // primary stub. A single edge, so the list renders as the one readable line.
    let BlockOutcome::Committed { result } = h
        .run(r#"return memory.get(PERSON_DAVE):outgoing("mentor_of")"#)
        .await
    else {
        panic!("expected commit");
    };
    assert_eq!(
        result,
        format!(
            "mentor_of → {}",
            MemoryName::from(Namespace::Person.with_name("erin")).as_str()
        )
    );

    // incoming: who mentors Dave — Frank, whose edge lands on the *other* stub, surfaced by traversal.
    let BlockOutcome::Committed { result } = h
        .run(r#"return memory.get(PERSON_DAVE):incoming("mentor_of")"#)
        .await
    else {
        panic!("expected commit");
    };
    assert_eq!(
        result,
        format!(
            "mentor_of ← {}",
            MemoryName::from(Namespace::Person.with_name("frank")).as_str()
        )
    );

    // links(): the whole relationship set across the identity — both mentor_of edges and works_at —
    // with the same_as edge holding the identity together excluded as internal plumbing.
    let BlockOutcome::Committed { result } =
        h.run(r#"return memory.get(PERSON_DAVE):links()"#).await
    else {
        panic!("expected commit");
    };
    assert!(
        result.contains(&format!(
            "mentor_of → {}",
            MemoryName::from(Namespace::Person.with_name("erin")).as_str()
        )),
        "{result}"
    );
    assert!(
        result.contains(&format!(
            "mentor_of ← {}",
            MemoryName::from(Namespace::Person.with_name("frank")).as_str()
        )),
        "{result}"
    );
    assert!(result.contains("works_at → company/hooli"), "{result}");
    assert!(
        !result.contains("same_as"),
        "the same_as plumbing must not surface as a relationship: {result}"
    );

    // A script branches on the structured fields, not only the rendered line — including `told_by`,
    // the teller behind the link (here the agent itself, "you", since these were agent-authored).
    let BlockOutcome::Committed { result } = h
        .run(
            r#"
        local out = memory.get(PERSON_DAVE):outgoing("mentor_of")
        return out[1].name .. " / " .. out[1].direction .. " / " .. out[1].source
            .. " / " .. out[1].told_by
        "#,
        )
        .await
    else {
        panic!("expected commit");
    };
    assert_eq!(
        result,
        format!(
            "{} / outgoing / agent / you",
            MemoryName::from(Namespace::Person.with_name("erin")).as_str()
        )
    );
}

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
async fn entries_render_as_their_text_and_concatenate() {
    let h = Harness::new();
    // An entry handle reads as its text: returned in a list (rendered for the model) and via `..`.
    let outcome = h
        .run(
            r#"
        local dave = memory.create(PERSON_DAVE)
        dave:append("climbs on Tuesdays", { visibility = "public" })
        local entries = dave:entries()
        return "first: " .. entries[1]
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert!(result.starts_with("first: climbs on Tuesdays"));
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
    assert_ne!(
        result, "<table>",
        "an undecorated table must not render opaquely"
    );
    assert!(
        result.contains("name"),
        "structure should be visible: {result}"
    );
    assert!(
        result.contains(MemoryName::from(Namespace::Person.with_name("dave")).as_str()),
        "values should be visible: {result}"
    );
    assert!(
        result.contains("visits"),
        "every key should be visible: {result}"
    );
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
        dave:link("mentor_of", erin)
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
        erin:link("mentored_by", dave)
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

#[tokio::test]
async fn memory_search_recalls_an_indexed_entry() {
    let h = Harness::with_retrieval();
    // Write a public fact, then embed it into the vector index.
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
    assert_eq!(
        result,
        MemoryName::from(Namespace::Person.with_name("dave")).as_str()
    );

    // Returning the result list renders as readable lines (each result's __tostring), not "<table>",
    // so the agent can read its own search back.
    let rendered = h
        .run(r#"return memory.search("An avid rock climber")"#)
        .await;
    let BlockOutcome::Committed { result } = rendered else {
        panic!("expected commit, got {rendered:?}");
    };
    assert!(
        result.contains(MemoryName::from(Namespace::Person.with_name("dave")).as_str()),
        "rendered: {result:?}"
    );
    assert!(!result.contains("<table>"), "rendered: {result:?}");
}

#[tokio::test]
async fn search_finds_a_renamed_person_by_an_old_name() {
    let h = Harness::with_retrieval();
    // A public fact that does *not* mention the name, then a rename — so only the alias-aware indexing
    // (the old name folded into the FTS) can make an old-name search find them.
    h.run(
        r#"
        local dave = memory.create(PERSON_DAVE)
        dave:append("Handles the deploys.", { by_agent = true, visibility = "public" })
        dave:rename(PERSON_SARAH)
        "#,
    )
    .await;
    h.index().await;

    // Searching by the former name surfaces the renamed person, flagged [formerly person/dave].
    let outcome = h
        .run(
            r#"
        local results = memory.search("Dave")
        if #results == 0 then return "none" end
        return results[1].name .. " | " .. tostring(results[1].marker)
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert!(
        result.starts_with(MemoryName::from(Namespace::Person.with_name("sarah")).as_str()),
        "{result}"
    );
    assert!(
        result.contains(&format!(
            "formerly {}",
            MemoryName::from(Namespace::Person.with_name("dave")).as_str()
        )),
        "{result}"
    );
}

#[tokio::test]
async fn print_output_is_surfaced_in_the_block_result() {
    // `print(...)` must feed back to the agent: Lua's default print writes to a process stdout the
    // model never reads, so an agent that inspects a value by printing it would see nothing. A block
    // whose final value is nil but which printed still returns the printed text.
    let h = Harness::new();
    let outcome = h
        .run(
            r#"
        print("hello")
        print("a", "b")
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert_eq!(result, "hello\na\tb");
}

#[tokio::test]
async fn printed_search_results_recall_the_fact() {
    // The recall failure mode: the agent searches, then `print`s each hit in a loop (so the block's
    // final value is nil) instead of returning the list. The printed names must still come back.
    let h = Harness::with_retrieval();
    h.run(
        r#"
        local dave = memory.create(PERSON_DAVE)
        dave:append("An avid rock climber", { by_agent = true, visibility = "public" })
        return "ok"
        "#,
    )
    .await;
    h.index().await;

    let outcome = h
        .run(
            r#"
        local results = memory.search("An avid rock climber")
        for _, res in ipairs(results) do
            print(res.name, res.description)
        end
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert!(
        result.contains(MemoryName::from(Namespace::Person.with_name("dave")).as_str()),
        "result: {result:?}"
    );
    assert!(!result.contains("<table>"), "result: {result:?}");
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

/// Register the merge-adjudication template directly, so the adjudication pass has its prompt without a
/// full genesis rollout (the scripted model returns a fixed verdict regardless of the prompt text).
fn register_adjudication_template(h: &Harness) {
    h.engine
        .store
        .lock()
        .as_mut()
        .append(
            h.clock.now(),
            vec![EventPayload::prompt_template_registered(
                PromptTemplateName::MergeAdjudication,
                1,
                "Decide whether two stubs are the same person, on the evidence.".to_owned(),
                EventSource::Orchestration,
            )],
        )
        .unwrap();
}

#[tokio::test]
async fn an_adjudicated_merge_links_two_stubs_on_accept() {
    // The agent proposes two stubs are one person; the off-hot-path adjudicator, accepting, authors the
    // same_as that merges them into one class (spec §Cross-platform identity → adjudicated merge).
    let h = Harness::new();
    register_adjudication_template(&h);
    h.run(
        r#"
        local a = memory.create(PERSON_DAVE_SLACK)
        a:append("Off sick the first week of March", { visibility = "private" })
        local b = memory.create(PERSON_DAVE_DISCORD)
        b:append("Out sick the week of March 3rd", { visibility = "private" })
        a:propose_merge(b)
        return "ok"
        "#,
    )
    .await;

    let model = ScriptedModel::new([Completion::Reply(
        r#"{"accepted": true, "rationale": "Both off sick the same week — an improbable coincidence."}"#
            .to_owned(),
    )]);
    h.adjudicate(&model).await;

    let graph = h.engine.graph.lock();
    let a = graph
        .memory_by_name(Namespace::Person.with_name("dave-slack"))
        .unwrap()
        .unwrap();
    let b = graph
        .memory_by_name(Namespace::Person.with_name("dave-discord"))
        .unwrap()
        .unwrap();
    let members = graph.class_members(a.id).unwrap();
    assert!(
        members.contains(&b.id),
        "the accepted merge should put both stubs in one same_as class, got {members:?}"
    );
}

#[tokio::test]
async fn a_refused_merge_leaves_the_stubs_distinct() {
    // On only a generic overlap the adjudicator refuses; no same_as is authored, the stubs stay in
    // separate classes, and the refusal is recorded for the operator.
    let h = Harness::new();
    register_adjudication_template(&h);
    h.run(
        r#"
        local a = memory.create(PERSON_SAM_SLACK)
        a:append("Is an engineer", { visibility = "public" })
        local b = memory.create(PERSON_SAM_DISCORD)
        b:append("Works in engineering", { visibility = "public" })
        a:propose_merge(b)
        return "ok"
        "#,
    )
    .await;

    let model = ScriptedModel::new([Completion::Reply(
        r#"{"accepted": false, "rationale": "Only a generic overlap; no specific coincidence."}"#
            .to_owned(),
    )]);
    h.adjudicate(&model).await;

    let graph = h.engine.graph.lock();
    let a = graph
        .memory_by_name(Namespace::Person.with_name("sam-slack"))
        .unwrap()
        .unwrap();
    let b = graph
        .memory_by_name(Namespace::Person.with_name("sam-discord"))
        .unwrap()
        .unwrap();
    assert!(
        !graph.class_members(a.id).unwrap().contains(&b.id),
        "a refused merge must leave the stubs in separate classes"
    );
    drop(graph);
    let events = h.events();
    assert!(
        events.iter().any(|e| matches!(
            &e.payload,
            EventPayload::MergeAdjudicated {
                accepted: false,
                ..
            }
        )),
        "a refusing verdict should be recorded for the operator"
    );
}

/// A fact on a memory the agent marked `high` volatility reads as `[stale]` once it ages past the
/// staleness horizon, so the agent hedges rather than asserting it as current; a default-volatility
/// memory's fact never goes stale. Staleness is age-based and independent of who is present.
#[tokio::test]
async fn a_high_volatility_fact_reads_stale_after_aging() {
    let h = Harness::new();
    h.run(
        r#"
        local d = memory.create(PERSON_DAVE)
        -- Classify volatility inline on the append (the ergonomic path).
        d:append("leads the Atlas project", { visibility = "public", volatility = "high" })
        local p = memory.create("project/atlas")
        p:append("the Atlas project ships in Q3", { visibility = "public" })
        "#,
    )
    .await;
    // Age past the 30-day staleness horizon.
    h.clock.advance_millis(40 * 86_400_000);

    let read = r#"
        local e = memory.get("MEM"):entries()[1]
        return tostring(e.stale) .. "|" .. tostring(e)
    "#;
    let BlockOutcome::Committed { result } = h
        .run(&read.replace(
            "MEM",
            MemoryName::from(Namespace::Person.with_name("dave")).as_str(),
        ))
        .await
    else {
        panic!("expected commit");
    };
    assert!(
        result.starts_with("true|") && result.contains("stale"),
        "the aged high-volatility fact should read stale: {result}"
    );
    let BlockOutcome::Committed { result } = h.run(&read.replace("MEM", "project/atlas")).await
    else {
        panic!("expected commit");
    };
    assert!(
        result.starts_with("false|"),
        "a default-volatility fact never goes stale: {result}"
    );
}

/// An `Attributed` fact — an ordinary thing a colleague relayed — survives the teller's absence: a
/// direct read by a present outsider sees it in full (unlike a confidence, which is withheld), so the
/// agent can still answer "what's Dave's role?" months later in another room. It reads as attributed,
/// carrying its provenance, never as a confidence.
#[tokio::test]
async fn an_attributed_fact_survives_the_teller_absence() {
    let h = Harness::new();
    h.run(r#"memory.create(PERSON_DAVE); memory.create(PERSON_ERIN)"#)
        .await;
    let id = |name: &str| {
        h.engine
            .graph
            .lock()
            .memory_by_name(MemoryName::new(name))
            .unwrap()
            .unwrap()
            .id
    };
    let (dave, erin) = (
        id(MemoryName::from(Namespace::Person.with_name("dave")).as_str()),
        id(MemoryName::from(Namespace::Person.with_name("erin")).as_str()),
    );

    // Erin, present, relays an ordinary fact about Dave (attributed) and a genuine confidence (private).
    h.run_as(
        Teller::Participant(erin),
        vec![erin],
        r#"
        memory.get(PERSON_DAVE):append("Engineering lead at Hooli", { visibility = "attributed" })
        memory.get(PERSON_DAVE):append("quietly interviewing elsewhere", { visibility = "private" })
        "#,
    )
    .await;

    // A different person (dave himself) present, the teller (erin) absent: the attributed fact stands
    // in full and reads as attributed; the confidence is withheld.
    let read = r#"
        local lines = {}
        for _, e in ipairs(memory.get(PERSON_DAVE):entries()) do
            lines[#lines + 1] = e.visibility .. "/" .. tostring(e.withheld) .. ":" .. e.text
        end
        return table.concat(lines, "|")
    "#;
    let BlockOutcome::Committed { result } = h.run_as(Teller::Agent, vec![dave], read).await else {
        panic!("expected commit");
    };
    assert!(
        result.contains("attributed/false:Engineering lead at Hooli"),
        "the attributed fact should survive the teller's absence, in full: {result}"
    );
    assert!(
        result.contains("private/true:(withheld") && !result.contains("interviewing elsewhere"),
        "the confidence should still be withheld from an outsider: {result}"
    );
}

/// A direct read withholds a confidence from a present audience that is not cleared to see it — the
/// same predicate search applies, now on `mem:entries`/`mem:history`. This closes the name-conflation
/// leak: reading `person/dave` while someone *other* than Dave is present must not hand over Dave's
/// confidence. A public fact is never withheld; with no one present the agent sees everything.
#[tokio::test]
async fn a_direct_read_withholds_a_confidence_from_a_present_outsider() {
    let h = Harness::new();
    h.run(r#"memory.create(PERSON_DAVE); memory.create(PERSON_ERIN)"#)
        .await;
    let id = |name: &str| {
        h.engine
            .graph
            .lock()
            .memory_by_name(MemoryName::new(name))
            .unwrap()
            .unwrap()
            .id
    };
    let (dave, erin) = (
        id(MemoryName::from(Namespace::Person.with_name("dave")).as_str()),
        id(MemoryName::from(Namespace::Person.with_name("erin")).as_str()),
    );

    // Dave, present, confides something private and states a public fact.
    h.run_as(
        Teller::Participant(dave),
        vec![dave],
        r#"
        memory.get(PERSON_DAVE):append("interviewing at a competitor", { visibility = "private" })
        memory.get(PERSON_DAVE):append("runs the Berlin marathon", { visibility = "public" })
        "#,
    )
    .await;

    // A read script that reports each entry as "<withheld>:<text>", oldest first.
    let read = r#"
        local lines = {}
        for _, e in ipairs(memory.get(PERSON_DAVE):entries()) do
            lines[#lines + 1] = tostring(e.withheld) .. ":" .. e.text
        end
        return table.concat(lines, "|")
    "#;

    // (a) Erin present, Dave absent: the confidence is withheld to a stub; the public fact stands.
    let BlockOutcome::Committed { result } = h.run_as(Teller::Agent, vec![erin], read).await else {
        panic!("expected commit");
    };
    assert!(
        result.contains("true:(withheld"),
        "the confidence should be withheld from Erin: {result}"
    );
    assert!(
        !result.contains("interviewing at a competitor"),
        "the confidence text must not reach a read while only Erin is present: {result}"
    );
    assert!(
        result.contains("false:runs the Berlin marathon"),
        "the public fact should stand: {result}"
    );

    // (b) Dave himself present: his own confidence surfaces in full.
    let BlockOutcome::Committed { result } = h.run_as(Teller::Agent, vec![dave], read).await else {
        panic!("expected commit");
    };
    assert!(
        result.contains("false:interviewing at a competitor"),
        "Dave present should see his own confidence: {result}"
    );

    // (c) No one present (a solo flush or maintenance read): the agent sees its whole memory.
    let BlockOutcome::Committed { result } = h.run_as(Teller::Agent, Vec::new(), read).await else {
        panic!("expected commit");
    };
    assert!(
        result.contains("false:interviewing at a competitor"),
        "a solo read is unredacted: {result}"
    );

    // (d) History redacts on the same rule, even though it shows superseded entries — Erin present.
    let history = r#"
        local lines = {}
        for _, e in ipairs(memory.get(PERSON_DAVE):history()) do
            lines[#lines + 1] = tostring(e.withheld) .. ":" .. e.text
        end
        return table.concat(lines, "|")
    "#;
    let BlockOutcome::Committed { result } = h.run_as(Teller::Agent, vec![erin], history).await
    else {
        panic!("expected commit");
    };
    assert!(
        result.contains("true:(withheld") && !result.contains("interviewing at a competitor"),
        "history withholds the confidence from Erin too: {result}"
    );
}

#[tokio::test]
async fn rename_keeps_the_memory_and_an_old_name_resolves_to_it() {
    let h = Harness::new();
    // A person with a fact, renamed to the name they now go by — all in one block.
    h.run(
        r#"
        local dave = memory.create(PERSON_DAVE)
        dave:append("climbs on Tuesdays", { visibility = "public" })
        dave:rename(PERSON_SARAH)
        "#,
    )
    .await;

    // The new handle resolves to the same memory, carrying the fact forward.
    let outcome = h
        .run(r#"return tostring(memory.get(PERSON_SARAH):entries()[1])"#)
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert!(result.contains("climbs on Tuesdays"), "{result}");

    // The old name still resolves — to the same memory, flagged (`former_handle`), under the current
    // handle — so someone using the old name is bridged to the renamed person rather than lost.
    let outcome = h
        .run(
            r#"
        local p = memory.get(PERSON_DAVE)
        return tostring(p ~= nil) .. " / " .. p.name .. " / " .. tostring(p.former_handle)
            .. " / " .. p.former_names[1]
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    // (The old-name lookup also emits its rename note ahead of the returned value, hence `contains`.)
    assert!(
        result.contains(&format!(
            "true / {} / {} / {}",
            MemoryName::from(Namespace::Person.with_name("sarah")).as_str(),
            MemoryName::from(Namespace::Person.with_name("dave")).as_str(),
            MemoryName::from(Namespace::Person.with_name("dave")).as_str()
        )),
        "{result}"
    );

    // Fetched by the *current* name, the memory still exposes its former names (so a read connects its
    // old-name content) but carries no `former_handle` (the lookup itself was not by an old name).
    let outcome = h
        .run(
            r#"
        local p = memory.get(PERSON_SARAH)
        return p.former_names[1] .. " / " .. tostring(p.former_handle)
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert_eq!(
        result,
        format!(
            "{} / nil",
            MemoryName::from(Namespace::Person.with_name("dave")).as_str()
        )
    );
}

#[tokio::test]
async fn an_old_name_lookup_announces_the_rename_in_the_output() {
    let h = Harness::new();
    h.run(
        r#"
        local dave = memory.create(PERSON_DAVE)
        dave:append("climbs on Tuesdays", { visibility = "public" })
        dave:rename(PERSON_SARAH)
        "#,
    )
    .await;

    // Looking the person up by their old name emits an active note into the agent's own output — so
    // however it goes on to inspect the handle, it cannot mistake the renamed node for a second person.
    let outcome = h
        .run(
            r#"
        local p = memory.get(PERSON_DAVE)
        print(p:entries()[1])
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert!(
        result.contains(&format!(
            r#"note: {:?} now goes by {:?} — the same person"#,
            MemoryName::from(Namespace::Person.with_name("dave")).as_str(),
            MemoryName::from(Namespace::Person.with_name("sarah")).as_str()
        )),
        "{result}"
    );
}

#[tokio::test]
async fn renaming_onto_an_occupied_handle_is_a_teachable_error() {
    let h = Harness::new();
    // Renaming one person onto another's handle is a collision — two people, not a rename.
    let outcome = h
        .run(
            r#"
        memory.create(PERSON_DAVE)
        memory.create(PERSON_ERIN)
        memory.get(PERSON_DAVE):rename(PERSON_ERIN)
        "#,
        )
        .await;
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = outcome else {
        panic!("expected a teachable error, got {outcome:?}");
    };
    assert!(
        message.contains(MemoryName::from(Namespace::Person.with_name("erin")).as_str()),
        "{message}"
    );
}
