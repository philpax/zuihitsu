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
    SessionId, Store, TagName, Teller, TemporalRef, TerminalCause, Timestamp, TurnId, TurnRole,
    Visibility,
    event::{ArbitrationResolution, EventPayload, EventSource, Initiation},
    ids::ConversationId,
    resolve_or_mint_conversation, turn_ref,
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
async fn a_date_object_prints_concatenates_and_stringifies() {
    // A date object renders as its ISO day through print/tostring, concatenates as that day from either
    // side, and answers :to_string() with the same — so "Reminder for " .. friday works instead of
    // erroring on a bare table, the crash the agent hit reaching for to_string/concat on a date.
    let h = Harness::new(); // clock at Monday 2026-06-08.
    let outcome = h
        .run(
            r#"
        local friday = calendar.next("friday")
        return tostring(friday)
            .. " | " .. ("on " .. friday)
            .. " | " .. (friday .. " it is")
            .. " | " .. friday:to_string()
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert_eq!(
        result,
        "2026-06-12 | on 2026-06-12 | 2026-06-12 it is | 2026-06-12"
    );
}

#[tokio::test]
async fn table_concat_renders_a_handle_list_rather_than_crashing() {
    // The recurring recall confusion: the agent reaches for table.concat on a reader's list as if its
    // elements were strings. Stock Lua 5.4 table.concat crashes on the handle tables ("invalid value
    // (at index 1) in table for 'concat'") since it joins only strings and numbers and invokes no
    // __tostring; the sandbox's lenient concat renders each element as its own text instead, so the
    // natural call joins the entries rather than terminating the block.
    let h = Harness::new();
    let outcome = h
        .run(
            r#"
        local dave = memory.create(PERSON_DAVE)
        dave:append("Met at the climbing gym", { visibility = "public" })
        dave:append("Got a new job at Hooli", { visibility = "public" })
        return table.concat(dave:entries(), " || ")
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert!(result.contains("Met at the climbing gym"), "got: {result}");
    assert!(result.contains("Got a new job at Hooli"), "got: {result}");
    assert!(
        result.contains(" || "),
        "the separator should join the two rendered entries, got: {result}"
    );
}

#[tokio::test]
async fn table_concat_joins_a_link_readers_list() {
    // The same lenient concat over a link reader's result: hub:links() returns link objects that print
    // as "relation → name", and table.concat joins them rather than crashing on the handle tables. The
    // link is committed in the first block, so the second block's :links() sees it.
    let h = Harness::new();
    h.run(
        r#"
        links.register({ name = "part_of", inverse = "contains", from_card = "many", to_card = "many" })
        local topic = memory.create(TOPIC_A)
        local ev = memory.create(EVENT_LAUNCH)
        ev:link("part_of", topic)
        return "ok"
        "#,
    )
    .await;
    let outcome = h
        .run(r#"return table.concat(memory.get(EVENT_LAUNCH):links(), "; ")"#)
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert!(
        result.contains("part_of"),
        "the link should render its relation, got: {result}"
    );
}

#[tokio::test]
async fn table_concat_on_a_reader_method_is_a_teachable_error() {
    // The field-vs-method slip: the agent passes hub.links (the method itself, a function) to
    // table.concat instead of hub:links() (its result). The lenient concat catches the non-table first
    // argument and redirects to the colon call rather than surfacing an opaque Lua error.
    let h = Harness::new();
    h.run(r#"memory.create(TOPIC_A)"#).await;
    let outcome = h
        .run(r#"return table.concat(memory.get(TOPIC_A).links, ", ")"#)
        .await;
    match outcome {
        BlockOutcome::Terminated(TerminalCause::Error(message)) => {
            assert!(
                message.contains("call it with a colon"),
                "message was: {message}"
            );
        }
        other => panic!("expected a teachable error, got {other:?}"),
    }
}

#[tokio::test]
async fn table_concat_preserves_stock_behavior_on_primitives() {
    // The lenient concat must not disturb ordinary joins: strings and numbers join as before, the
    // default separator is empty, and the optional i/j range still selects a sub-span.
    let h = Harness::new();
    let outcome = h
        .run(
            r#"
        return table.concat({ "a", "b", "c" }, "-")
            .. " | " .. table.concat({ 1, 2, 3 })
            .. " | " .. table.concat({ "a", "b", "c", "d" }, ",", 2, 3)
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert_eq!(result, "a-b-c | 123 | b,c");
}

#[tokio::test]
async fn calendar_on_accepts_a_date_object() {
    // calendar.on takes a date object as readily as a "YYYY-MM-DD" string, so the calendar's own
    // today()/next() return values feed straight into its sibling query rather than crashing on the
    // string-argument conversion.
    let h = Harness::new(); // clock at Monday 2026-06-08.
    h.run(
        r#"
        local d = memory.create(EVENT_CLEANING)
        d:append("dentist", { visibility = "public", occurred_at = calendar.today() })
        "#,
    )
    .await;
    let outcome = h.run(r#"return #calendar.on(calendar.today())"#).await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert_eq!(result, "1");
}

#[tokio::test]
async fn occurred_at_accepts_a_date_object_in_a_day_field() {
    // A date object stands in for the string inside a { day = ... } tagged table, so the agent's
    // natural { day = calendar.next("friday") } deserializes rather than failing the string field.
    let h = Harness::new(); // clock at Monday 2026-06-08.
    let outcome = h
        .run(
            r#"
        local ev = memory.create(EVENT_BOARD_UPDATE)
        ev:append("Send the board update", { occurred_at = { day = calendar.next("friday") }, visibility = "public" })
        return ev:entries()
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert!(
        result.contains("[2026-06-12"),
        "the nested date object should land as the day occurrence, got: {result}"
    );
}

#[tokio::test]
async fn occurred_at_accepts_a_range_from_date_objects_or_strings() {
    // A ranged occurred_at accepts date objects *or* bare "YYYY-MM-DD" strings for its endpoints,
    // converting each to the day's bounding instant (start of the first day, end of the last), so the
    // agent builds a date range from calendar handles or from the days it already has as strings, instead
    // of hand-computing millisecond timestamps or being turned away with an i64 deserialize error.
    let h = Harness::new();
    let outcome = h
        .run(
            r#"
        local ev = memory.create(EVENT_CLEANING)
        ev:append("Conference", {
            visibility = "public",
            occurred_at = { range = { start = calendar.date("2026-06-10"), ["end"] = calendar.date("2026-06-12") } }
        })
        return ev:entries()
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    // The range spans all of both boundary days and renders as its start–end.
    assert!(
        result.contains("2026-06-10 – 2026-06-12"),
        "the range from date objects should render its span, got: {result}"
    );

    // And it denormalized to the range's midpoint sort, end to end.
    let ev = h
        .engine
        .graph
        .lock()
        .memory_by_name(Namespace::Event.with_name("cleaning"))
        .unwrap()
        .unwrap();
    let entries = h.engine.graph.lock().entries_local(ev.id).unwrap();
    assert_eq!(entries.len(), 1);
    let start = Timestamp::from_millis(20_614 * 86_400_000); // 2026-06-10 midnight UTC.
    let end = Timestamp::from_millis(20_616 * 86_400_000 + 86_400_000 - 1); // 2026-06-12, last ms.
    let expected = TemporalRef::Range { start, end }
        .bounds(None, BEFORE_AFTER_EPSILON_MILLIS)
        .sort;
    assert_eq!(entries[0].occurred_sort, expected);

    // The same endpoints given as bare "YYYY-MM-DD" strings coerce identically — start to the first ms,
    // end to the last — so a script that writes the days as strings lands the same range rather than
    // failing the endpoints' i64 deserialize.
    let string_outcome = h
        .run(
            r#"
        local ev = memory.create(EVENT_LAUNCH)
        ev:append("Conference", {
            visibility = "public",
            occurred_at = { range = { start = "2026-06-10", ["end"] = "2026-06-12" } }
        })
        return ev:entries()
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = string_outcome else {
        panic!("expected commit, got {string_outcome:?}");
    };
    assert!(
        result.contains("2026-06-10 – 2026-06-12"),
        "the range from date strings should render the same span, got: {result}"
    );
    let launch = h
        .engine
        .graph
        .lock()
        .memory_by_name(Namespace::Event.with_name("launch"))
        .unwrap()
        .unwrap();
    let launch_entries = h.engine.graph.lock().entries_local(launch.id).unwrap();
    assert_eq!(launch_entries.len(), 1);
    assert_eq!(launch_entries[0].occurred_sort, expected);
}

#[tokio::test]
async fn occurred_at_accepts_an_instant_as_a_date_string() {
    // A bare "YYYY-MM-DD" string in the instant position coerces to the day's first millisecond rather
    // than failing the i64 deserialize the Instant variant expects, so an agent that hands a date where a
    // millisecond timestamp is wanted is met by the coercion, not a crash. It coerces like a range start.
    let h = Harness::new();
    let outcome = h
        .run(
            r#"
        local ev = memory.create(EVENT_CLEANING)
        ev:append("Kickoff", { visibility = "public", occurred_at = { instant = "2026-06-10" } })
        return "ok"
        "#,
        )
        .await;
    assert!(
        matches!(outcome, BlockOutcome::Committed { .. }),
        "an instant given as a date string must commit, got {outcome:?}"
    );
    let ev = h
        .engine
        .graph
        .lock()
        .memory_by_name(Namespace::Event.with_name("cleaning"))
        .unwrap()
        .unwrap();
    let entries = h.engine.graph.lock().entries_local(ev.id).unwrap();
    assert_eq!(entries.len(), 1);
    // The string landed at the day's first millisecond, as a precise Instant (not a whole-day Day).
    let expected = TemporalRef::Instant(Timestamp::from_millis(20_614 * 86_400_000)) // 2026-06-10 midnight UTC.
        .bounds(None, BEFORE_AFTER_EPSILON_MILLIS)
        .sort;
    assert_eq!(entries[0].occurred_sort, expected);
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
async fn calendar_overdue_surfaces_past_excludes_future_and_recurring() {
    // "What should I be on top of?" a day after a reminder's due date: calendar.overdue surfaces the
    // dated occurrence that has already passed, while calendar.on(today)/calendar.upcoming — looking
    // at today and ahead — would miss it. A future occurrence and a recurring one are excluded (a
    // recurrence's next instance is always ahead, so it is never overdue). Clock at Monday 2026-06-08.
    let h = Harness::new();
    h.run(
        r#"
        local past = memory.create(EVENT_CLEANING)
        past:append("dentist", { visibility = "public", occurred_at = { day = "2026-06-05" } })
        local future = memory.create(EVENT_LAUNCH)
        future:append("launch", { visibility = "public", occurred_at = { day = "2026-06-11" } })
        local rec = memory.create(EVENT_STANDUP)
        rec:append("standup", { visibility = "public", occurred_at = { recurring = "FREQ=WEEKLY" } })
        "#,
    )
    .await;
    let outcome = h
        .run(
            r#"
        local past = memory.get(EVENT_CLEANING)
        local future = memory.get(EVENT_LAUNCH)
        local rec = memory.get(EVENT_STANDUP)
        local saw_past, saw_future, saw_rec = false, false, false
        for _, m in ipairs(calendar.overdue()) do
            if m.id == past.id then saw_past = true end
            if m.id == future.id then saw_future = true end
            if m.id == rec.id then saw_rec = true end
        end
        return tostring(saw_past) .. "," .. tostring(saw_future) .. "," .. tostring(saw_rec)
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert_eq!(result, "true,false,false");
}

#[tokio::test]
async fn calendar_overdue_respects_the_lookback_bound() {
    // The default lookback is a fortnight, so an occurrence 19 days back stays hidden until the window
    // is widened — overdue answers "what did I recently miss?", not "everything ever dated". Clock at
    // Monday 2026-06-08, so 2026-05-20 is 19 days ago.
    let h = Harness::new();
    h.run(
        r#"
        local stale = memory.create(EVENT_ALL_HANDS)
        stale:append("all hands", { visibility = "public", occurred_at = { day = "2026-05-20" } })
        "#,
    )
    .await;
    let outcome = h
        .run(
            r#"
        local target = memory.get(EVENT_ALL_HANDS)
        local function sees(list)
            for _, m in ipairs(list) do
                if m.id == target.id then return "found" end
            end
            return "missing"
        end
        return sees(calendar.overdue()) .. "," .. sees(calendar.overdue({ within = "30 days" }))
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    // Outside the default 14-day window, inside a widened 30-day one.
    assert_eq!(result, "missing,found");
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
async fn a_retired_seed_relation_is_gated_by_the_registry_not_the_vocabulary() {
    // `_session_carryover`/`_session_carries` was retired from the genesis seed set (issue #21), yet
    // `RelationName::new` still maps both strings to their typed variants so OLD logs' events keep
    // materializing. The concern: does that hardcoded vocabulary mapping bypass the registry gate in
    // the fresh-instance link path and silently materialize a link under a relation the system
    // retired? It must not — the registry (the materialized `relations` table), not the string
    // recognizer, is the gate. A fresh Harness skips genesis, so its registry never learns
    // `_session_carryover`: linking under it must fail exactly like any unregistered name.
    let h = Harness::new();
    h.run(r#"memory.create(TOPIC_A)"#).await;
    let outcome = h
        .run(r#"memory.get(TOPIC_A):link("_session_carryover", memory.get(TOPIC_A))"#)
        .await;
    match outcome {
        BlockOutcome::Terminated(TerminalCause::Error(message)) => {
            assert!(
                message.contains("unknown relation"),
                "a retired seed relation must be a teachable unknown-relation error, got: {message}"
            );
        }
        other => panic!("expected a teachable unknown-relation error, got {other:?}"),
    }
    // The rejected block committed nothing: no `LinkCreated` under the retired relation reached the
    // log, so the vocabulary mapping did not smuggle an edge past the registry.
    assert!(
        !h.events().iter().any(|event| matches!(
            &event.payload,
            EventPayload::LinkCreated { relation, .. }
                if *relation == RelationName::SessionCarryover
        )),
        "no LinkCreated under the retired relation must land"
    );

    // The gate is the registry, not a string blocklist: an agent can still deliberately register a
    // fresh relation and link under it. Generic runtime registration keeps working.
    h.run(r#"memory.create(TOPIC_ALPHA)"#).await;
    let outcome = h
        .run(
            r#"links.register({ name = "carries_over", inverse = "carried_from", from_card = "many", to_card = "many" })
               memory.get(TOPIC_A):link("carries_over", memory.get(TOPIC_ALPHA))"#,
        )
        .await;
    assert!(
        matches!(outcome, BlockOutcome::Committed { .. }),
        "a freshly registered relation must link and commit, got {outcome:?}"
    );
    assert!(
        h.events().iter().any(|event| matches!(
            &event.payload,
            EventPayload::LinkCreated { relation, .. }
                if relation.as_str() == "carries_over"
        )),
        "the deliberately registered relation's link must materialize"
    );
}

#[tokio::test]
async fn link_and_unlink_resolve_a_name_string_target() {
    // A name string in place of a handle is looked up, not rejected with a type error that would roll
    // the whole block back — the cascade that silently dropped a co-located private write (#43). This
    // block links via a string *and* appends a confidence in one go; both must survive together. Unlink
    // shares the same resolution seam, so a name string clears the edge too.
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

    // Unlink through the same seam: a name string clears the edge just as it made it.
    let unlink_outcome = h
        .run(r#"memory.get(PERSON_DAVE):unlink("knows", PERSON_ERIN)"#)
        .await;
    assert!(
        matches!(unlink_outcome, BlockOutcome::Committed { .. }),
        "a string-target unlink must commit, got {unlink_outcome:?}"
    );
    let BlockOutcome::Committed { result } = h
        .run(r#"return memory.get(PERSON_DAVE):outgoing("knows")"#)
        .await
    else {
        panic!("expected a committed read");
    };
    assert!(
        !result.contains("erin"),
        "the knows edge should be gone after unlinking by name, got: {result:?}"
    );
}

#[tokio::test]
async fn link_to_an_unknown_name_teaches_creation() {
    // A name string that names no memory is a teachable miss — it says the name is unknown and points at
    // creating it or checking the casing, rather than lecturing about handles, so the agent's fix is to
    // create the memory (or correct a typo), not to reach for a handle it does not have.
    let h = Harness::new();
    h.run(r#"memory.create(PERSON_DAVE)"#).await;
    let outcome = h
        .run(r#"memory.get(PERSON_DAVE):link("knows", "person/nobody")"#)
        .await;
    match outcome {
        BlockOutcome::Terminated(TerminalCause::Error(message)) => {
            assert!(
                message.contains("no memory named \"person/nobody\"")
                    && message.contains("create it first")
                    && message.contains("casing"),
                "the unknown name should teach creation/casing, got: {message}"
            );
        }
        other => panic!("expected a teachable unknown-name error, got {other:?}"),
    }
}

#[tokio::test]
async fn a_memory_handle_renders_its_link_neighborhood() {
    // A topic hub prints its links line, so a recall that fetches the hub sees the spokes its
    // decisions live on — the linked events — rather than reading only the hub's own entries and
    // dropping a fact that sits one link away (the hub-and-spoke recall gap). The links are committed
    // in one block, then the hub is fetched in the next (block.links reflects committed state).
    let h = Harness::new();
    h.run(
        r#"
        links.register({ name = "part_of", inverse = "contains", from_card = "many", to_card = "many" })
        local topic = memory.create(TOPIC_MIGRATION, "The billing migration")
        local ship = memory.create(EVENT_LAUNCH, "Ship the migration")
        ship:link("part_of", topic)
        "#,
    )
    .await;

    let BlockOutcome::Committed { result } = h.run(r#"return memory.get(TOPIC_MIGRATION)"#).await
    else {
        panic!("expected a committed read");
    };
    assert!(
        result.contains("links:"),
        "the handle should render a links line, got: {result}"
    );
    assert!(
        result.contains("part_of")
            && result.contains(MemoryName::from(Namespace::Event.with_name("launch")).as_str()),
        "the links line should name the relation and the linked event, got: {result}"
    );
}

#[tokio::test]
async fn a_dated_link_target_shows_its_occurrence_on_the_handle() {
    // A dated spoke carries its date onto the hub's links line (the same `[when …]` phrasing a search
    // hit uses), so relaying the recap from the handle keeps the *when* without a separate read.
    let h = Harness::new();
    h.run(
        r#"
        links.register({ name = "part_of", inverse = "contains", from_card = "many", to_card = "many" })
        local topic = memory.create(TOPIC_MIGRATION, "The billing migration")
        local ship = memory.create(EVENT_LAUNCH)
        ship:append("Ship it", { visibility = "public", occurred_at = { day = "2026-08-01" } })
        ship:link("part_of", topic)
        "#,
    )
    .await;

    let BlockOutcome::Committed { result } = h.run(r#"return memory.get(TOPIC_MIGRATION)"#).await
    else {
        panic!("expected a committed read");
    };
    assert!(
        result.contains("[when 2026-08-01]"),
        "the dated spoke should show its occurrence on the links line, got: {result}"
    );
}

#[tokio::test]
async fn the_neighborhood_line_caps_and_notes_the_remainder() {
    // A busy hub does not flood the transcript: the links line shows the first several and elides the
    // rest with a `(+N more)` note. Nine events linked to the topic exceeds the cap of eight.
    let h = Harness::new();
    h.run(
        r#"
        links.register({ name = "part_of", inverse = "contains", from_card = "many", to_card = "many" })
        local topic = memory.create(TOPIC_MIGRATION, "The billing migration")
        for i = 1, 9 do
            local ev = memory.create("event/spoke-" .. i)
            ev:link("part_of", topic)
        end
        "#,
    )
    .await;

    let BlockOutcome::Committed { result } = h.run(r#"return memory.get(TOPIC_MIGRATION)"#).await
    else {
        panic!("expected a committed read");
    };
    assert!(
        result.contains("(+1 more)"),
        "the links line should cap and note the elided remainder, got: {result}"
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
            // The wording points at the fetch-or-make idiom for when existence is uncertain.
            assert!(
                message.contains("memory.get_or_create"),
                "message was: {message}"
            );
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
async fn get_or_create_returns_the_existing_memory_without_clobbering_it() {
    // `memory.get_or_create` on a name that already exists returns that memory as it stands: the
    // content argument is ignored, so an existing memory's entries are never overwritten or appended
    // to by a fetch-that-happened-to-pass-content.
    let h = Harness::new();
    h.run(r#"memory.create(TOPIC_CLIMBING, "Met at the climbing gym")"#)
        .await;
    let outcome = h
        .run(
            r#"local made = memory.get_or_create(TOPIC_CLIMBING, "this content must be ignored")
               return made:entries()"#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    // The original entry is intact and the ignored content never landed.
    assert!(result.contains("Met at the climbing gym"), "{result}");
    assert!(!result.contains("this content must be ignored"), "{result}");

    // Exactly one entry on the memory — the fetch appended nothing.
    let climbing = h
        .engine
        .graph
        .lock()
        .memory_by_name(Namespace::Topic.with_name("climbing"))
        .unwrap()
        .unwrap();
    assert_eq!(
        h.engine
            .graph
            .lock()
            .entries_local(climbing.id)
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn get_or_create_creates_the_memory_when_it_is_absent() {
    // With no such memory yet, `memory.get_or_create` creates it with the given first entry — the
    // create half of the fetch-or-make idiom.
    let h = Harness::new();
    let outcome = h
        .run(
            r#"local made = memory.get_or_create(TOPIC_SOURDOUGH, "A naturally leavened bread")
               return made:entries()"#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert!(result.contains("naturally leavened"), "{result}");

    // The memory was committed and carries its first entry.
    let sourdough = h
        .engine
        .graph
        .lock()
        .memory_by_name(Namespace::Topic.with_name("sourdough"))
        .unwrap()
        .unwrap();
    assert_eq!(
        h.engine
            .graph
            .lock()
            .entries_local(sourdough.id)
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn assigning_occurred_at_on_a_handle_is_a_teachable_error() {
    // A memory handle is a read-only view: `m.occurred_at = ...` was a silent no-op that misled the
    // agent into thinking a date landed (the traced gate slip). It now raises a teachable error naming
    // the operations that actually persist a dated fact.
    let h = Harness::new();
    let outcome = h
        .run(
            r#"local dave = memory.create(PERSON_DAVE)
               dave.occurred_at = calendar.date("2027-03-15")
               return "unreached""#,
        )
        .await;
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = outcome else {
        panic!("expected a teachable error, got {outcome:?}");
    };
    assert!(
        message.contains("occurred_at is not assignable"),
        "{message}"
    );
    // The guidance names the right moves: append with occurred_at in its opts, or revise.
    assert!(message.contains("occurred_at in its opts"), "{message}");
    assert!(message.contains("revise"), "{message}");
}

#[tokio::test]
async fn assigning_a_field_on_an_entry_handle_is_a_teachable_error() {
    // The read-only guard covers entry handles too — assigning to an entry's field does nothing, so it
    // raises rather than silently dropping the write.
    let h = Harness::new();
    let outcome = h
        .run(
            r#"local topic = memory.create(TOPIC_CLIMBING, "a fact")
               local es = topic:entries()
               es[1].occurred_at = calendar.date("2027-03-15")
               return "unreached""#,
        )
        .await;
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = outcome else {
        panic!("expected a teachable error, got {outcome:?}");
    };
    assert!(
        message.contains("occurred_at is not assignable"),
        "{message}"
    );
}

#[tokio::test]
async fn calling_a_method_with_a_dot_suggests_the_colon() {
    // `dave.append(...)` binds the string to `self`, which used to fail with mlua's opaque "error
    // converting Lua string to table". It now raises a teachable error naming the method and the colon
    // call.
    let h = Harness::new();
    let outcome = h
        .run(
            r#"local dave = memory.create(PERSON_DAVE)
               dave.append("a fact", { visibility = "public" })
               return "unreached""#,
        )
        .await;
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = outcome else {
        panic!("expected a teachable error, got {outcome:?}");
    };
    assert!(message.contains("is a method"), "{message}");
    assert!(message.contains("colon"), "{message}");
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
    // each result is a { name, description, score, marker?, snippet? } table.
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
async fn memory_search_carries_a_dated_hits_occurrence() {
    // A scheduled fact's date rides on the hit, so a recall relayed from the result — the line or the
    // `occurred_at` field — keeps the *when* without a separate `entries()` read. The regression: a
    // search hit dropped the resolved date, and recaps rendered from it lost the day.
    let h = Harness::with_retrieval();
    let seeded = h
        .run(
            r#"
        local ev = memory.create(EVENT_LAUNCH)
        ev:append("shipping the billing migration on Friday the 17th",
            { by_agent = true, visibility = "public", occurred_at = { day = "2026-07-17" } })
        return "ok"
        "#,
        )
        .await;
    assert!(matches!(seeded, BlockOutcome::Committed { .. }));
    h.index().await;

    // The result carries the occurrence as the same tagged table `append` takes, so a script reads the
    // date off the hit directly.
    let field = h
        .run(
            r#"
        local results = memory.search("shipping the billing migration")
        if #results == 0 then return "none" end
        return results[1].occurred_at.day
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = field else {
        panic!("expected commit, got {field:?}");
    };
    assert_eq!(result, "2026-07-17");

    // And the rendered line shows the date, so a recap relayed from the printed result keeps it.
    let rendered = h
        .run(r#"return memory.search("shipping the billing migration")"#)
        .await;
    let BlockOutcome::Committed { result } = rendered else {
        panic!("expected commit, got {rendered:?}");
    };
    assert!(result.contains("[when 2026-07-17]"), "rendered: {result:?}");
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
async fn an_empty_search_query_fails_fast_and_teaches_the_listing_shape() {
    // An empty (or whitespace) query has nothing to match on — the agent reaching for it wants to
    // *list* a namespace, which search does not do. The guard short-circuits before the embedder is
    // ever touched: even on a retrieval-less harness, where "anything" reports search unavailable, an
    // empty query reports the query problem instead, and points at the nearest legitimate shape (a
    // real query narrowed by the namespace option, since no listing affordance exists).
    let h = Harness::new();
    let outcome = h.run(r#"return memory.search("   ")"#).await;
    match outcome {
        BlockOutcome::Terminated(TerminalCause::Error(message)) => {
            assert!(
                message.contains("needs a query") && message.contains("cannot list a namespace"),
                "message was: {message}"
            );
            assert!(
                message.contains("namespace = \"topic/\""),
                "the error names the namespace-narrowed shape: {message}"
            );
            assert!(
                !message.contains("unavailable"),
                "the empty-query guard precedes the embedder path: {message}"
            );
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

#[tokio::test]
async fn a_proposals_rationale_reaches_the_adjudication_prompt() {
    // The rationale the agent states with propose_merge rides the MergeProposed event and is injected
    // into the adjudicator's prompt as the proposer's claim — so the adjudicator weighs the stated
    // grounds against the two stubs' persisted entries rather than seeing only the entries.
    let h = Harness::new();
    register_adjudication_template(&h);
    h.run(
        r#"
        local a = memory.create(PERSON_DAVE_SLACK)
        a:append("At the Reykjavik conference in June", { visibility = "public" })
        local b = memory.create(PERSON_DAVE_DISCORD)
        b:append("Was on a research trip to Iceland", { visibility = "public" })
        a:propose_merge(b, { rationale = "Both mention the same volcanology trip and the same wedding." })
        return "ok"
        "#,
    )
    .await;

    // The stated grounds ride the event.
    assert!(
        h.events().iter().any(|e| matches!(
            &e.payload,
            EventPayload::MergeProposed { rationale: Some(text), .. }
                if text == "Both mention the same volcanology trip and the same wedding."
        )),
        "the rationale must ride the MergeProposed event"
    );

    let model = ScriptedModel::new([Completion::Reply(
        r#"{"accepted": false, "rationale": "Weighed the claim against the facts; not enough."}"#
            .to_owned(),
    )]);
    h.adjudicate(&model).await;

    // ... and reach the adjudicator's prompt, labeled as the proposer's claim rather than as evidence.
    let prompts: Vec<String> = model
        .recorded_messages()
        .iter()
        .flatten()
        .map(|message| message.content.clone())
        .collect();
    assert!(
        prompts.iter().any(|p| {
            p.contains("Both mention the same volcanology trip and the same wedding.")
                && p.contains("their claim, not evidence")
        }),
        "the adjudication prompt must carry the proposer's stated rationale as a claim: {prompts:?}"
    );
}

/// A fact on a memory the agent marked `high` volatility reads as `[stale — no newer entry]` once it
/// ages past the staleness horizon, so the agent hedges rather than asserting it as current — the
/// marker says the fact aged out *and nothing replaced it*, so the agent reconfirms rather than
/// hunting for a fresher version. A default-volatility memory's fact never goes stale. Staleness is
/// age-based and independent of who is present.
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
        result.starts_with("true|") && result.contains("stale — no newer entry"),
        "the aged high-volatility fact should read `stale — no newer entry`: {result}"
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

/// A superseded aged high-volatility entry — surfaced only by `mem:history`, never a live read —
/// does *not* carry the stale marker: its newer version sits right beside it in the same list, so
/// marking it "no newer entry" would lie. The live tail that aged out with nothing replacing it still
/// reads stale. This is the render distinction the marker's wording promises: `stale — no newer entry`
/// only ever rides an unreplaced fact.
#[tokio::test]
async fn a_superseded_aged_entry_is_not_marked_stale_in_history() {
    let h = Harness::new();
    h.run(
        r#"
        local d = memory.create(PERSON_DAVE)
        d:append("leads the Atlas project", { visibility = "public", volatility = "high" })
        "#,
    )
    .await;
    // Age past the 30-day horizon so the first entry is stale, then supersede it with a newer fact
    // that is itself fresh.
    h.clock.advance_millis(40 * 86_400_000);
    let dave = MemoryName::from(Namespace::Person.with_name("dave"))
        .as_str()
        .to_owned();
    h.run(
        &r#"
        local d = memory.get("MEM")
        local old = d:entries()[1]
        d:revise(old, "now leads the Borealis project", { visibility = "public", volatility = "high" })
        "#
        .replace("MEM", &dave),
    )
    .await;

    let read = r#"
        local d = memory.get("MEM")
        local live = {}
        for _, e in ipairs(d:entries()) do
            live[#live + 1] = tostring(e)
        end
        local past = {}
        for _, e in ipairs(d:history()) do
            past[#past + 1] = tostring(e.stale) .. ":" .. tostring(e)
        end
        return "LIVE=" .. table.concat(live, "|") .. "~~HIST=" .. table.concat(past, "|")
    "#;
    let BlockOutcome::Committed { result } = h.run(&read.replace("MEM", &dave)).await else {
        panic!("expected commit");
    };
    // The live read shows only the fresh successor, unmarked.
    assert!(
        result.contains("LIVE=") && result.contains("now leads the Borealis project"),
        "the live read should surface the fresh successor: {result}"
    );
    assert!(
        !result.split("~~HIST=").next().unwrap().contains("stale"),
        "the live read has no aged-out entry, so nothing is marked stale: {result}"
    );
    // History keeps the superseded entry, but it is not marked stale — its successor is right there.
    let history = result.split("~~HIST=").nth(1).unwrap();
    assert!(
        history.contains("false:") && history.contains("leads the Atlas project"),
        "history keeps the superseded entry, unmarked (it has a successor): {result}"
    );
    assert!(
        !history.contains("stale"),
        "a superseded aged entry must not read stale — there IS a newer entry: {result}"
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

// The `convo.turn` transcript-link resolver under the audience rule (spec §Transcripts). A turn
// resolves iff everyone present where the resolver runs was in that moment's audience — its session's
// participants plus any mid-session joiners up to it. The matrix below drives every branch: a plain
// resolve, the audience-mismatch warning (same room, later session a newcomer joined), cross-room
// resolves permitted by the loosening (solo DM and two-person DM where all attended), the two-person
// DM where one did not attend, the unknown- and malformed-id errors, a mid-session join filtering the
// window, the `ref` field round-tripping through `turn_ref::scan`, and the feature-off nil-call.

/// A person stub, materialized so the resolver can render the speaker's conversational handle.
fn person(store: &mut MemoryStore, clock: &ManualClock, handle: &str) -> MemoryId {
    let id = MemoryId::generate();
    store
        .append(
            clock.now(),
            vec![EventPayload::memory_created(
                id,
                Namespace::Person.with_name(handle),
            )],
        )
        .unwrap();
    id
}

/// A `SessionStarted` opening `session` in `conversation` with `participants` as its audience.
fn session_started(
    conversation: ConversationId,
    session: SessionId,
    participants: Vec<MemoryId>,
    started_at: Timestamp,
) -> EventPayload {
    EventPayload::SessionStarted {
        conversation,
        id: session,
        participants,
        started_at,
        seeded_from_turn: None,
        brief: String::new(),
    }
}

/// A `ParticipantJoined` recording `participant` arriving mid-`session`.
fn participant_joined(
    conversation: ConversationId,
    session: SessionId,
    participant: MemoryId,
) -> EventPayload {
    EventPayload::ParticipantJoined {
        conversation,
        session,
        participant,
        at_turn: TurnId::generate(),
    }
}

fn turn_event(
    conversation: ConversationId,
    turn_id: TurnId,
    role: TurnRole,
    text: &str,
    participant: Option<MemoryId>,
) -> EventPayload {
    EventPayload::ConversationTurn {
        conversation,
        turn_id,
        role,
        text: text.to_owned(),
        participant,
        initiation: Initiation::Responding,
        produced_by: None,
    }
}

/// A block context whose present set drives the audience rule the resolver applies.
fn resolver_context(present_set: Vec<MemoryId>) -> BlockContext {
    BlockContext {
        teller: Teller::Agent,
        authority: Authority::Platform,
        turn_id: TurnId::generate(),
        block_timeout: TEST_BLOCK_TIMEOUT,
        max_block_attempts: TEST_MAX_BLOCK_ATTEMPTS,
        present_set,
        dry_run: false,
    }
}

/// Boot an engine over a store the caller has appended to, materializing the graph.
fn resolver_engine(store: MemoryStore, clock: &ManualClock) -> Arc<Engine> {
    let mut graph = Graph::open_in_memory().unwrap();
    graph.materialize_from(&store).unwrap();
    Engine::new(Box::new(store), graph, Box::new(clock.clone()))
}

#[tokio::test]
async fn convo_turn_resolves_within_audience_and_carries_a_ref() {
    let mut store = MemoryStore::new();
    let clock = ManualClock::new(common::time::EARLY);
    let conversation = resolve_or_mint_conversation(
        &mut store,
        &clock,
        &Graph::open_in_memory().unwrap(),
        &ConversationLocator::new("discord", "planning"),
    )
    .unwrap();
    let sarah = person(&mut store, &clock, "sarah");
    let session = SessionId::generate();

    let before = TurnId::generate();
    let focus = TurnId::generate();
    let after = TurnId::generate();
    store
        .append(
            clock.now(),
            vec![
                session_started(conversation, session, vec![sarah], clock.now()),
                turn_event(
                    conversation,
                    before,
                    TurnRole::Participant,
                    "Kicking off Q3 planning.",
                    Some(sarah),
                ),
                turn_event(
                    conversation,
                    focus,
                    TurnRole::Participant,
                    "We ship Meridian on August 14th.",
                    Some(sarah),
                ),
                turn_event(
                    conversation,
                    after,
                    TurnRole::Agent,
                    "Noted — Meridian on the 14th.",
                    None,
                ),
            ],
        )
        .unwrap();

    let session_vm = Session::new(conversation, InstanceFeatures::default());
    let engine = resolver_engine(store, &clock);

    // Sarah is present, and she was the moment's audience — it resolves with its window.
    let outcome = session_vm
        .execute(
            &engine,
            &resolver_context(vec![sarah]),
            &format!(r#"return convo.turn("{}")"#, focus.0),
        )
        .await
        .unwrap();
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert!(
        result.contains("We ship Meridian on August 14th."),
        "{result}"
    );
    assert!(result.contains("Kicking off Q3 planning."), "{result}");
    assert!(result.contains("Noted — Meridian on the 14th."), "{result}");
    assert!(result.contains("sarah"), "{result}");

    // The `ref` field is the canonical token, and it round-trips back to the focal id through the one
    // parser — the agent cites by copying it rather than hand-assembling syntax.
    let outcome = session_vm
        .execute(
            &engine,
            &resolver_context(vec![sarah]),
            &format!(r#"return convo.turn("{}").ref"#, focus.0),
        )
        .await
        .unwrap();
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert_eq!(turn_ref::extract_ids(&result), vec![focus]);
}

#[tokio::test]
async fn convo_turn_warns_when_a_newcomer_was_not_in_the_audience() {
    // Same room, two sessions: session 1 is Maya and Tom on something sensitive; session 2 is Maya and
    // a newcomer Sam. Resolving the session-1 moment while Sam is present must warn, not replay.
    let mut store = MemoryStore::new();
    let clock = ManualClock::new(common::time::EARLY);
    let conversation = resolve_or_mint_conversation(
        &mut store,
        &clock,
        &Graph::open_in_memory().unwrap(),
        &ConversationLocator::new("discord", "leads"),
    )
    .unwrap();
    let maya = person(&mut store, &clock, "maya");
    let tom = person(&mut store, &clock, "tom");
    let sam = person(&mut store, &clock, "sam");

    let session_one = SessionId::generate();
    let sensitive = TurnId::generate();
    store
        .append(
            clock.now(),
            vec![
                session_started(conversation, session_one, vec![maya, tom], clock.now()),
                turn_event(
                    conversation,
                    sensitive,
                    TurnRole::Participant,
                    "The layoffs land Friday — keep it off the record for now.",
                    Some(tom),
                ),
                EventPayload::session_ended(conversation, session_one),
            ],
        )
        .unwrap();
    let session_two = SessionId::generate();
    store
        .append(
            clock.now(),
            vec![session_started(
                conversation,
                session_two,
                vec![maya, sam],
                clock.now(),
            )],
        )
        .unwrap();

    let session_vm = Session::new(conversation, InstanceFeatures::default());
    let engine = resolver_engine(store, &clock);
    let outcome = session_vm
        .execute(
            &engine,
            &resolver_context(vec![maya, sam]),
            &format!(r#"return convo.turn("{}")"#, sensitive.0),
        )
        .await
        .unwrap();
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = outcome else {
        panic!("expected an audience-mismatch warning, got {outcome:?}");
    };
    assert!(message.contains("audience"), "{message}");
    assert!(message.contains("memory"), "{message}");
    // The refusal never carries the withheld content or the id-is-unknown wording.
    assert!(
        !message.contains("layoffs"),
        "must not leak content: {message}"
    );
    assert!(
        !message.contains("no turn"),
        "audience-mismatch is worded distinctly from not-found: {message}"
    );
}

#[tokio::test]
async fn convo_turn_resolves_cross_room_for_a_solo_dm() {
    // A group room the requester attended, and a solo DM with just the requester. The loosening lets
    // the DM resolve the group-room moment the requester was party to.
    let mut store = MemoryStore::new();
    let clock = ManualClock::new(common::time::EARLY);
    let graph = Graph::open_in_memory().unwrap();
    let room = resolve_or_mint_conversation(
        &mut store,
        &clock,
        &graph,
        &ConversationLocator::new("discord", "leads"),
    )
    .unwrap();
    let dm = resolve_or_mint_conversation(
        &mut store,
        &clock,
        &graph,
        &ConversationLocator::new("direct", "maya"),
    )
    .unwrap();
    let maya = person(&mut store, &clock, "maya");
    let tom = person(&mut store, &clock, "tom");

    let room_session = SessionId::generate();
    let moment = TurnId::generate();
    store
        .append(
            clock.now(),
            vec![
                session_started(room, room_session, vec![maya, tom], clock.now()),
                turn_event(
                    room,
                    moment,
                    TurnRole::Participant,
                    "We ship Meridian on August 14th.",
                    Some(maya),
                ),
            ],
        )
        .unwrap();
    // The DM's own session — only Maya present.
    let dm_session = SessionId::generate();
    store
        .append(
            clock.now(),
            vec![session_started(dm, dm_session, vec![maya], clock.now())],
        )
        .unwrap();

    let session_vm = Session::new(dm, InstanceFeatures::default());
    let engine = resolver_engine(store, &clock);
    let outcome = session_vm
        .execute(
            &engine,
            &resolver_context(vec![maya]),
            &format!(r#"return convo.turn("{}")"#, moment.0),
        )
        .await
        .unwrap();
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert!(
        result.contains("We ship Meridian on August 14th."),
        "{result}"
    );
}

#[tokio::test]
async fn convo_turn_two_person_dm_resolves_only_when_both_attended() {
    // A group-room moment attended by Alice and Bob (not Carol). A two-person DM of Alice+Bob resolves
    // it; a two-person DM of Alice+Carol does not — Carol was not in that moment's audience.
    let mut store = MemoryStore::new();
    let clock = ManualClock::new(common::time::EARLY);
    let graph = Graph::open_in_memory().unwrap();
    let room = resolve_or_mint_conversation(
        &mut store,
        &clock,
        &graph,
        &ConversationLocator::new("discord", "leads"),
    )
    .unwrap();
    let alice = person(&mut store, &clock, "alice");
    let bob = person(&mut store, &clock, "bob");
    let carol = person(&mut store, &clock, "carol");

    let room_session = SessionId::generate();
    let moment = TurnId::generate();
    store
        .append(
            clock.now(),
            vec![
                session_started(room, room_session, vec![alice, bob], clock.now()),
                turn_event(
                    room,
                    moment,
                    TurnRole::Participant,
                    "Budget sign-off is with finance until Thursday.",
                    Some(alice),
                ),
            ],
        )
        .unwrap();

    let session_vm = Session::new(room, InstanceFeatures::default());
    let engine = resolver_engine(store, &clock);

    // Both attended — resolves.
    let outcome = session_vm
        .execute(
            &engine,
            &resolver_context(vec![alice, bob]),
            &format!(r#"return convo.turn("{}")"#, moment.0),
        )
        .await
        .unwrap();
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert!(
        result.contains("Budget sign-off is with finance until Thursday."),
        "{result}"
    );

    // Carol was not in that moment's audience — the same id warns.
    let outcome = session_vm
        .execute(
            &engine,
            &resolver_context(vec![alice, carol]),
            &format!(r#"return convo.turn("{}")"#, moment.0),
        )
        .await
        .unwrap();
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = outcome else {
        panic!("expected an audience-mismatch warning, got {outcome:?}");
    };
    assert!(message.contains("audience"), "{message}");
}

#[tokio::test]
async fn convo_turn_unknown_and_malformed_ids_are_distinct_errors() {
    let mut store = MemoryStore::new();
    let clock = ManualClock::new(common::time::EARLY);
    let conversation = resolve_or_mint_conversation(
        &mut store,
        &clock,
        &Graph::open_in_memory().unwrap(),
        &ConversationLocator::new("discord", "planning"),
    )
    .unwrap();

    let session_vm = Session::new(conversation, InstanceFeatures::default());
    let engine = resolver_engine(store, &clock);

    // A well-formed but never-recorded id is not-found — worded distinctly from the audience warning.
    let unknown = TurnId::generate();
    let outcome = session_vm
        .execute(
            &engine,
            &resolver_context(Vec::new()),
            &format!(r#"return convo.turn("{}")"#, unknown.0),
        )
        .await
        .unwrap();
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = outcome else {
        panic!("expected a teachable error, got {outcome:?}");
    };
    assert!(message.contains("no turn"), "{message}");
    assert!(!message.contains("audience"), "{message}");

    // A malformed id is teachable and distinctly worded again.
    let outcome = session_vm
        .execute(
            &engine,
            &resolver_context(Vec::new()),
            r#"return convo.turn("not-a-ulid")"#,
        )
        .await
        .unwrap();
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = outcome else {
        panic!("expected a teachable error, got {outcome:?}");
    };
    assert!(message.contains("invalid turn id"), "{message}");
}

#[tokio::test]
async fn convo_turn_window_filters_a_mid_session_join() {
    // Maya and Tom open a session; a turn is recorded; then Sam joins mid-session and another turn
    // lands. Resolving the post-join turn while Maya and Sam are present drops the pre-join neighbor
    // from the window — Sam was not in the audience of that earlier turn.
    let mut store = MemoryStore::new();
    let clock = ManualClock::new(common::time::EARLY);
    let conversation = resolve_or_mint_conversation(
        &mut store,
        &clock,
        &Graph::open_in_memory().unwrap(),
        &ConversationLocator::new("discord", "leads"),
    )
    .unwrap();
    let maya = person(&mut store, &clock, "maya");
    let tom = person(&mut store, &clock, "tom");
    let sam = person(&mut store, &clock, "sam");

    let session = SessionId::generate();
    let pre_join = TurnId::generate();
    let post_join = TurnId::generate();
    store
        .append(
            clock.now(),
            vec![
                session_started(conversation, session, vec![maya, tom], clock.now()),
                turn_event(
                    conversation,
                    pre_join,
                    TurnRole::Participant,
                    "Only the two of us know about the reorg.",
                    Some(tom),
                ),
                participant_joined(conversation, session, sam),
                turn_event(
                    conversation,
                    post_join,
                    TurnRole::Participant,
                    "Welcome Sam, glad you could join.",
                    Some(maya),
                ),
            ],
        )
        .unwrap();

    let session_vm = Session::new(conversation, InstanceFeatures::default());
    let engine = resolver_engine(store, &clock);
    let outcome = session_vm
        .execute(
            &engine,
            &resolver_context(vec![maya, sam]),
            &format!(r#"return convo.turn("{}")"#, post_join.0),
        )
        .await
        .unwrap();
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert!(result.contains("Welcome Sam"), "{result}");
    assert!(
        !result.contains("reorg"),
        "the pre-join neighbor is filtered from the window: {result}"
    );
}

#[tokio::test]
async fn convo_turn_is_absent_when_transcripts_are_disabled() {
    let disabled = InstanceFeatures {
        transcripts: false,
        ..Default::default()
    };
    let h = Harness::with_features(disabled);
    let outcome = h
        .run(&format!(r#"return convo.turn("{}")"#, TurnId::generate().0))
        .await;
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = outcome else {
        panic!("expected a nil-call error, got {outcome:?}");
    };
    assert!(
        message.contains("nil"),
        "a disabled convo.turn should surface a nil-call error, got: {message}"
    );
}
