use super::*;
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
            assert!(message.contains("no entry"), "message was: {message}");
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
async fn retract_drops_an_entry_from_live_reads_but_keeps_it_in_history_with_its_reason() {
    let h = Harness::new();
    // Record a fact, then withdraw it outright with a reason. The block's own live read reflects the
    // withdrawal (read-your-writes); history keeps the tombstone.
    let outcome = h
        .run(
            r#"
        local dave = memory.create(PERSON_DAVE)
        local fact = dave:append("Dave plays the cello", { visibility = "public" })
        dave:retract(fact, "filed on the wrong person")
        return "live=" .. #dave:entries() .. " history=" .. #dave:history()
        "#,
        )
        .await;

    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert!(result.starts_with("live=0 history=1"), "result: {result}");
    assert!(
        result.contains(&format!(
            "retracted an entry on {}",
            MemoryName::from(Namespace::Person.with_name("dave")).as_str()
        )),
        "the commit summary names the retraction: {result}"
    );

    // Committed and projected: the live read is empty, history keeps the tombstone with its reason.
    let dave = h
        .engine
        .graph
        .lock()
        .memory_by_name(Namespace::Person.with_name("dave"))
        .unwrap()
        .unwrap();
    assert!(
        h.engine
            .graph
            .lock()
            .entries_local(dave.id)
            .unwrap()
            .is_empty()
    );
    let history = h.engine.graph.lock().class_history(dave.id).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(
        history[0].retracted_reason.as_deref(),
        Some("filed on the wrong person")
    );
}

#[tokio::test]
async fn retract_requires_a_reason() {
    let h = Harness::new();
    // A retraction with no stated reason is unauditable, so an empty reason is a teachable error and
    // nothing commits.
    let outcome = h
        .run(
            r#"
        local dave = memory.create(PERSON_DAVE)
        local fact = dave:append("a fact", { visibility = "public" })
        dave:retract(fact, "")
        "#,
        )
        .await;
    match outcome {
        BlockOutcome::Terminated(TerminalCause::Error(message)) => {
            assert!(
                message.contains("non-empty reason") || message.contains("unauditable"),
                "message was: {message}"
            );
        }
        other => panic!("expected a teachable error, got {other:?}"),
    }
    // The rejected block committed nothing.
    assert!(
        h.engine
            .graph
            .lock()
            .memory_by_name(Namespace::Person.with_name("dave"))
            .unwrap()
            .is_none(),
        "the whole block was discarded"
    );
}

#[tokio::test]
async fn the_correction_two_step_leaves_no_live_residue_on_the_wrong_memory() {
    let h = Harness::new();
    // The motivating trace: a role fact lands on the wrong person, is noticed, and is corrected — not
    // by superseding in place (which cannot move it), but by retracting it here and re-asserting it on
    // the right person with the original teller. The wrong memory ends with no live residue; the fact
    // lives on the right one.
    let outcome = h
        .run(
            r#"
        local david = memory.create("person/david")
        local wrong = david:append("Leads the design team", { by_agent = true, visibility = "public" })
        -- Noticed: that is Davina's role, not David's. Retract it here and re-assert it on her memory.
        david:retract(wrong, "that is Davina's role, not David's")
        local davina = memory.create("person/davina")
        davina:append("Leads the design team", { by_agent = true, visibility = "public" })
        return "david=" .. #david:entries() .. " davina=" .. #davina:entries()
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert!(result.starts_with("david=0 davina=1"), "result: {result}");

    let graph = h.engine.graph.lock();
    let david = graph
        .memory_by_name(MemoryName::new("person/david"))
        .unwrap()
        .unwrap();
    let davina = graph
        .memory_by_name(MemoryName::new("person/davina"))
        .unwrap()
        .unwrap();
    // No live residue on David; the fact lives on Davina; David's history keeps the tombstone with why.
    assert!(graph.entries_local(david.id).unwrap().is_empty());
    let davina_live: Vec<String> = graph
        .entries_local(davina.id)
        .unwrap()
        .into_iter()
        .map(|e| e.text)
        .collect();
    assert_eq!(davina_live, ["Leads the design team"]);
    let david_history = graph.class_history(david.id).unwrap();
    assert_eq!(
        david_history[0].retracted_reason.as_deref(),
        Some("that is Davina's role, not David's")
    );
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

#[tokio::test]
async fn every_read_surface_renders_an_entry_with_its_full_id() {
    // The rendered entry line leads with the entry's full id — the stable handle the agent addresses a
    // correction by — on all three agent-invoked read surfaces (entries, history, details), so the id
    // is discoverable in the output the agent already reads rather than only on the handle field.
    let h = Harness::new();
    let outcome = h
        .run(
            r#"
        local dave = memory.create(PERSON_DAVE)
        local e = dave:append("Leads the design team", { visibility = "public" })
        local id = e.id
        local line = tostring(dave:entries()[1])
        -- The id leads the bracket, labelled: "[id <id> · public · from person/dave] Leads…".
        local leads_bracket = line:sub(1, #id + 5) == "[id " .. id .. " "
        local in_history = tostring(dave:history()[1]):find(id, 1, true) ~= nil
        local in_details = dave:details():find(id, 1, true) ~= nil
        if leads_bracket and in_history and in_details and #id == 26 then
            return "ALL_SURFACES_CARRY_THE_FULL_ID"
        end
        return "MISSING: " .. line
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert!(
        result.starts_with("ALL_SURFACES_CARRY_THE_FULL_ID"),
        "entries, history, and details must each render the full entry id: {result}"
    );
}

#[tokio::test]
async fn supersede_accepts_a_unique_id_prefix() {
    // A unique prefix of the entry's rendered id resolves to that entry, so the agent can correct a
    // fact by the id it just read rather than by holding the handle. The 22-char prefix is past the
    // id's shared timestamp run, so it names exactly the old entry.
    let h = Harness::new();
    let outcome = h
        .run(
            r#"
        local dave = memory.create(PERSON_DAVE)
        local old = dave:append("Dave works at Hooli", { visibility = "public" })
        local new = dave:append("Dave works at Pied Piper", { visibility = "public" })
        dave:supersede(string.sub(old.id, 1, 22), new)
        return "live=" .. #dave:entries() .. " text=" .. tostring(dave:entries()[1])
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert!(
        result.starts_with("live=1"),
        "the prefix superseded old: {result}"
    );
    assert!(
        result.contains("Pied Piper"),
        "only the correction is live: {result}"
    );
}

#[tokio::test]
async fn retract_accepts_a_unique_id_prefix() {
    // Retract resolves a unique id prefix exactly as supersede does.
    let h = Harness::new();
    let outcome = h
        .run(
            r#"
        local dave = memory.create(PERSON_DAVE)
        local fact = dave:append("Dave plays the cello", { visibility = "public" })
        dave:retract(string.sub(fact.id, 1, 22), "filed on the wrong person")
        return "live=" .. #dave:entries()
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert!(
        result.starts_with("live=0"),
        "the prefix retracted the fact: {result}"
    );
}

#[tokio::test]
async fn supersede_accepts_the_full_id_string() {
    // The full id string works as well as the handle it was read from.
    let h = Harness::new();
    let outcome = h
        .run(
            r#"
        local dave = memory.create(PERSON_DAVE)
        local old = dave:append("Dave works at Hooli", { visibility = "public" })
        local new = dave:append("Dave works at Pied Piper", { visibility = "public" })
        dave:supersede(old.id, new)
        return "live=" .. #dave:entries()
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert!(
        result.starts_with("live=1"),
        "the full id superseded old: {result}"
    );
}

#[tokio::test]
async fn an_ambiguous_id_prefix_lists_the_candidates() {
    // A prefix that matches more than one entry of the class is a teachable error naming each
    // candidate by id and a text snippet, so the agent can disambiguate with a longer prefix. The
    // shared leading run of two same-block ids (their common timestamp) is such a prefix.
    let h = Harness::new();
    let outcome = h
        .run(
            r#"
        local dave = memory.create(PERSON_DAVE)
        local a = dave:append("first fact", { visibility = "public" })
        local b = dave:append("second fact", { visibility = "public" })
        local n = 0
        while n < #a.id and a.id:sub(n + 1, n + 1) == b.id:sub(n + 1, n + 1) do
            n = n + 1
        end
        dave:supersede(a.id:sub(1, n), a)
        "#,
        )
        .await;
    match outcome {
        BlockOutcome::Terminated(TerminalCause::Error(message)) => {
            assert!(
                message.contains("matches more than one entry"),
                "message names the ambiguity: {message}"
            );
            assert!(
                message.contains("first fact") && message.contains("second fact"),
                "message lists both candidates' snippets: {message}"
            );
        }
        other => panic!("expected an ambiguity error, got {other:?}"),
    }
}

#[tokio::test]
async fn an_unknown_entry_id_is_a_teachable_error() {
    // A syntactically valid id that names no entry of this memory is the UnknownEntry error, nothing
    // commits, and the block is discarded.
    let h = Harness::new();
    let outcome = h
        .run(
            r#"
        local dave = memory.create(PERSON_DAVE)
        local fact = dave:append("a real fact", { visibility = "public" })
        dave:supersede("00000000000000000000000000", fact)
        "#,
        )
        .await;
    match outcome {
        BlockOutcome::Terminated(TerminalCause::Error(message)) => {
            assert!(message.contains("no entry"), "message was: {message}");
        }
        other => panic!("expected a teachable error, got {other:?}"),
    }
}

#[tokio::test]
async fn an_entry_id_corrects_where_a_text_scan_misses_on_case() {
    // The traced failure: the agent text-scanned to re-find an entry to retract
    // (entry.text:find("leads the design team")) and missed, because the fact was recorded
    // "Leads the design team" with a capital L, so the retraction was skipped entirely. Addressing the
    // entry by its rendered id succeeds where the case-sensitive scan returns nil.
    let h = Harness::new();
    let outcome = h
        .run(
            r#"
        local dave = memory.create(PERSON_DAVE)
        dave:append("Leads the design team", { visibility = "public" })
        local entry = dave:entries()[1]
        local text_scan = entry.text:find("leads the design team")
        dave:retract(entry.id, "role changed")
        return "text_scan=" .. tostring(text_scan) .. " live=" .. #dave:entries()
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert!(
        result.starts_with("text_scan=nil live=0"),
        "the case-sensitive text scan misses but the id retracts: {result}"
    );
}
