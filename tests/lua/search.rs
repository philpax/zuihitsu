use super::*;

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
async fn memory_search_carries_salient_relations_on_a_hit() {
    // A hit passively carries its most salient relations — people first — so a search for a recurring
    // event reveals the cast already participating in it, the recognition signal that steers a recall
    // toward reusing the memory it found rather than minting a name-guessed duplicate.
    let h = Harness::with_retrieval();
    let seeded = h
        .run(
            r#"
        links.register({ name = "participates_in", inverse = "has_participant", from_card = "many", to_card = "many" })
        local club = memory.create(EVENT_STANDUP, "The weekly standup")
        local marcus = memory.create(PERSON_MARCUS)
        local erin = memory.create(PERSON_ERIN)
        links.create(marcus, "participates_in", club)
        links.create(erin, "participates_in", club)
        return "ok"
        "#,
        )
        .await;
    assert!(matches!(seeded, BlockOutcome::Committed { .. }));
    h.index().await;

    // The structural field: the agent reads the cast off the hit as { relation, name, direction }.
    let field = h
        .run(
            r#"
        local results = memory.search("The weekly standup")
        for _, r in ipairs(results) do
            if r.name == EVENT_STANDUP and r.relations then
                local first = r.relations[1]
                return first.relation .. " " .. first.direction .. " " .. first.name
            end
        end
        return "none"
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = field else {
        panic!("expected commit, got {field:?}");
    };
    assert!(
        result.starts_with(&format!(
            "participates_in incoming {}",
            Namespace::Person.prefix()
        )),
        "the structural relation reads people-first as {{ relation, name, direction }}: {result}"
    );

    // The rendered line shows the relations inline in the `relation ← name` house style.
    let rendered = h.run(r#"return memory.search("The weekly standup")"#).await;
    let BlockOutcome::Committed { result } = rendered else {
        panic!("expected commit, got {rendered:?}");
    };
    assert!(
        result.contains("participates_in ← "),
        "the hit line renders its salient relations: {result:?}"
    );
    assert!(
        result.contains(MemoryName::from(Namespace::Person.with_name("marcus")).as_str())
            && result.contains(MemoryName::from(Namespace::Person.with_name("erin")).as_str()),
        "the relations line names the participants: {result:?}"
    );
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
async fn a_search_hit_still_renders_with_its_score_and_snippet() {
    // Making a hit a usable handle changes method/field ACCESS only — how it prints is unchanged. The
    // rendered line still leads with the name and score and carries the matched-content snippet.
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

    let rendered = h
        .run(r#"return memory.search("An avid rock climber")"#)
        .await;
    let BlockOutcome::Committed { result } = rendered else {
        panic!("expected commit, got {rendered:?}");
    };
    assert!(
        result.contains("(score "),
        "the hit still renders its score: {result:?}"
    );
    assert!(
        result.contains(r#"match: "An avid rock climber""#),
        "the hit still renders its snippet: {result:?}"
    );
    assert!(!result.contains("<table>"), "rendered: {result:?}");
}

#[tokio::test]
async fn a_search_hit_appends_directly() {
    // A hit is also a usable memory handle: `hits[1]:append(…)` writes to the found memory without a
    // `memory.get(hits[1].name)` round-trip. The method locks the memory itself, so a read-only search
    // followed by a write on its hit commits cleanly.
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

    let appended = h
        .run(
            r#"
        local results = memory.search("An avid rock climber")
        if #results == 0 then return "none" end
        results[1]:append("Also boulders on weekends", { by_agent = true, visibility = "public" })
        return "wrote"
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = appended else {
        panic!("expected commit, got {appended:?}");
    };
    assert!(result.contains("wrote"), "{result:?}");

    // The entry landed on Dave: a fresh block reads it back off the memory itself.
    let read_back = h.run(r#"return memory.get(PERSON_DAVE):details()"#).await;
    let BlockOutcome::Committed { result } = read_back else {
        panic!("expected commit, got {read_back:?}");
    };
    assert!(
        result.contains("Also boulders on weekends"),
        "the hit's append should land on Dave: {result:?}"
    );
}

#[tokio::test]
async fn a_search_hit_reads_details_and_keeps_its_own_fields() {
    // Handle methods and lazy fields fall through to the memory-handle metatable, so `hit:details()`
    // reads the full record — while the hit's OWN carried fields (`score`, `snippet`) still read as the
    // search machinery set them, never shadowed by the handle fallback.
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
        if #results == 0 then return "none" end
        local hit = results[1]
        local detail = hit:details()
        return `score:{hit.score > 0} snippet:{hit.snippet} detail:{detail:find("An avid rock climber") ~= nil}`
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert!(
        result.contains("score:true"),
        "the hit keeps its score: {result}"
    );
    assert!(
        result.contains("snippet:An avid rock climber"),
        "the hit keeps its own snippet, unshadowed: {result}"
    );
    assert!(
        result.contains("detail:true"),
        "the handle method reads the full record through the fallback: {result}"
    );
}

#[tokio::test]
async fn memory_get_on_a_search_hit_resolves_the_same_memory() {
    // The dual-accept `memory.get` reads a handle's `id` field; a hit carries it too, so
    // `memory.get(hit)` resolves the same memory the hit points at.
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
        if #results == 0 then return "none" end
        return memory.get(results[1]).name
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
}

#[tokio::test]
async fn assigning_to_a_search_hit_field_is_a_teachable_error() {
    // A hit shares the read-only `__newindex` posture of every handle: assigning to a field it does not
    // carry — `occurred_at`, the traced date footgun — does not persist, so it raises the teachable
    // error rather than silently doing nothing. (The hit's own carried fields are raw entries the
    // guard cannot intercept; the posture is verified through a key the guard actually sees.)
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
        results[1].occurred_at = calendar.date("2027-03-15")
        return "unreached"
        "#,
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
