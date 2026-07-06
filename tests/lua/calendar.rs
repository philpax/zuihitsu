use super::*;

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
async fn calendar_windows_take_a_bare_duration_string() {
    // `calendar.upcoming("31 days")` is the shape the agent naturally writes — the bare duration
    // stands for `{ within = … }` on both window functions, and a wrong-typed window teaches the
    // accepted shapes instead of failing the mlua conversion opaquely.
    let h = Harness::new(); // clock at Monday 2026-06-08.
    h.run(
        r#"
        local e = memory.create(EVENT_ALL_HANDS)
        e:append("all hands", { visibility = "public", occurred_at = { day = "2026-07-01" } })
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
        return sees(calendar.upcoming("31 days")) .. "," .. sees(calendar.upcoming("7 days"))
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    // July 1st is 23 days out: inside a 31-day window, outside a 7-day one.
    assert_eq!(result, "found,missing");

    let outcome = h.run(r#"return calendar.upcoming(31)"#).await;
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = outcome else {
        panic!("expected a teachable failure, got {outcome:?}");
    };
    assert!(
        message.contains("duration") && message.contains("31 days"),
        "the window error should teach the accepted shapes, got: {message}"
    );
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
