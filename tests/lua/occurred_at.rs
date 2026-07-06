use super::*;

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
        ev:append("Penciled in by Marcus", { visibility = "public", occurred_at = { day = "2027-03-15" } })
        return ev:entries()
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit");
    };
    assert!(
        result.contains("[2027-03-15") && result.contains("Penciled in by Marcus"),
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
async fn memory_create_accepts_occurred_at_in_its_options_table() {
    // `memory.create` accepts an options table as its third argument and flows it through to the first
    // entry exactly like `mem:append`, so a reminder created in one call keeps its `occurred_at` and
    // fires.
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
async fn occurred_at_accepts_a_bare_date_string() {
    // The intuitive `occurred_at = "2026-06-03"` — a bare top-level date string, not a tagged table —
    // lands the same `Day` occurrence as `{ day = "2026-06-03" }`, instead of failing serde with a raw
    // enum-variant list. It is the shape the agent reaches for first.
    let h = Harness::new();
    let outcome = h
        .run(
            r#"
        local ev = memory.create(EVENT_CLEANING)
        ev:append("Scheduled cleaning", { visibility = "public", occurred_at = "2026-06-03" })
        return "ok"
        "#,
        )
        .await;
    assert!(
        matches!(outcome, BlockOutcome::Committed { .. }),
        "a bare date string must commit, got {outcome:?}"
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
    // Identical to the tagged `{ day = "2026-06-03" }` form: a whole-day Day, sorting at its noon.
    let expected = TemporalRef::Day(CivilDate("2026-06-03".into()))
        .bounds(None, BEFORE_AFTER_EPSILON_MILLIS)
        .sort;
    assert_eq!(entries[0].occurred_sort, expected);
    assert!(expected.is_some());
}

#[tokio::test]
async fn an_opts_table_reused_across_appends_keeps_its_fields() {
    // The boundary resolves occurred_at and told_by out of the opts table itself, and must do so
    // without mutating the agent's table: a script that builds one opts table and reuses it across
    // appends keeps the occurrence on every entry, not just the first.
    let h = Harness::new();
    let outcome = h
        .run(
            r#"
        local ev = memory.create(EVENT_CLEANING)
        local opts = { visibility = "public", occurred_at = "2026-06-03" }
        ev:append("First pass", opts)
        ev:append("Second pass", opts)
        return ev:entries()
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert_eq!(
        result.matches("[2026-06-03").count(),
        2,
        "both appends should carry the shared opts table's occurrence, got: {result}"
    );
}

#[tokio::test]
async fn a_bogus_occurred_at_teaches_the_accepted_shapes() {
    // An occurred_at value that names no occurrence — here a non-date string — raises a teachable error
    // naming the accepted shapes (a bare date, a date object, or a tagged table), never leaking serde's
    // enum-variant phrasing ("unknown variant, expected instant/day/range/…").
    let h = Harness::new();
    let outcome = h
        .run(
            r#"
        local ev = memory.create(EVENT_CLEANING)
        ev:append("Scheduled cleaning", { visibility = "public", occurred_at = "sometime next week" })
        return "unreached"
        "#,
        )
        .await;
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = outcome else {
        panic!("expected a teachable error, got {outcome:?}");
    };
    assert!(
        message.contains("occurred_at does not name an occurrence"),
        "the error should name the failing option: {message}"
    );
    // It teaches the shapes the agent can use instead.
    assert!(
        message.contains("YYYY-MM-DD") && message.contains("{ day =") && message.contains("range"),
        "the error should enumerate the accepted shapes: {message}"
    );
    // And it does not leak serde's enum-variant soup.
    assert!(
        !message.contains("unknown variant") && !message.contains("expected one of"),
        "the error must not surface serde's variant list: {message}"
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
