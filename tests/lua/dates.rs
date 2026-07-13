use super::*;

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
        result.contains("2026-06-12"),
        "the computed Friday should land as the occurrence, got: {result}"
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
