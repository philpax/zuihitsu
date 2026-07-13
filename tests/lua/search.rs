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
    // followed by a write on its hit commits cleanly. The query names Dave, so the fuzzy-write guard
    // passes it — a direct write through a hit is allowed precisely when the search named the referent.
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
        local results = memory.search("Dave")
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
async fn a_fuzzy_write_through_a_mismatched_hit_is_refused() {
    // The run-8 repro: David's memory mentions Davina, so a naive search for "Davina" surfaces the
    // person/david hit. Taking it as her and appending her role through it — the
    // `if #hits == 0 then create else hits[1]` idiom — must be refused, naming both the query and the
    // handle it landed on, and committing nothing.
    let h = Harness::with_retrieval();
    let seeded = h
        .run(
            r#"
        local david = memory.create("person/david")
        david:append("Introduced to Davina at the design sync", { by_agent = true, visibility = "public" })
        return "ok"
        "#,
        )
        .await;
    assert!(matches!(seeded, BlockOutcome::Committed { .. }));
    h.index().await;

    let outcome = h
        .run(
            r#"
        local hits = memory.search("Davina")
        if #hits == 0 then return "none" end
        hits[1]:append("Leads the design team", { by_agent = true, visibility = "public" })
        return "wrote"
        "#,
        )
        .await;
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = outcome else {
        panic!("expected the fuzzy-write guard to refuse, got {outcome:?}");
    };
    assert!(
        message.contains("search for \"Davina\"") && message.contains("names person/david"),
        "the error names the query and the handle: {message}"
    );
    assert!(
        message.contains("candidate, not a match"),
        "the error teaches a hit is a candidate: {message}"
    );
    assert!(
        message.contains("memory.get(\"person/david\")")
            && message.contains("memory.list(\"person/dav")
            && message.contains("memory.create(\"person/davina\")"),
        "the error offers the three-way get/list/create confirmation: {message}"
    );

    // Nothing landed on David: his record still holds only the seeded entry.
    let read_back = h
        .run(r#"return memory.get("person/david"):details()"#)
        .await;
    let BlockOutcome::Committed { result } = read_back else {
        panic!("expected commit, got {read_back:?}");
    };
    assert!(
        !result.contains("Leads the design team"),
        "the refused write must not have committed: {result}"
    );
}

#[tokio::test]
async fn a_retract_through_a_mismatched_hit_is_refused() {
    // Retract is a guarded content writer like append, so withdrawing an entry through a fuzzy hit the
    // query did not name is refused before it commits — the guards compose. A correction that mistook
    // the referent must not launder itself through a retraction any more than through an append.
    let h = Harness::with_retrieval();
    let seeded = h
        .run(
            r#"
        local david = memory.create("person/david")
        david:append("Introduced to Davina at the design sync", { by_agent = true, visibility = "public" })
        return "ok"
        "#,
        )
        .await;
    assert!(matches!(seeded, BlockOutcome::Committed { .. }));
    h.index().await;

    let outcome = h
        .run(
            r#"
        local hits = memory.search("Davina")
        if #hits == 0 then return "none" end
        local entry = hits[1]:history()[1]
        hits[1]:retract(entry, "wrong person")
        return "wrote"
        "#,
        )
        .await;
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = outcome else {
        panic!("expected the fuzzy-write guard to refuse, got {outcome:?}");
    };
    assert!(
        message.contains("search for \"Davina\"") && message.contains("names person/david"),
        "the error names the query and the handle: {message}"
    );

    // Nothing was withdrawn: David's seeded entry is still live.
    let read_back = h
        .run(r#"return memory.get("person/david"):details()"#)
        .await;
    let BlockOutcome::Committed { result } = read_back else {
        panic!("expected commit, got {read_back:?}");
    };
    assert!(
        result.contains("Introduced to Davina at the design sync"),
        "the refused retract must not have committed: {result}"
    );
}

#[tokio::test]
async fn an_exact_token_hit_writes_through_directly() {
    // A search whose word names the hit passes the guard: "David" names person/david, so appending
    // through the hit commits without a memory.get round-trip.
    let h = Harness::with_retrieval();
    h.run(
        r#"
        local david = memory.create("person/david")
        david:append("Leads the platform team", { by_agent = true, visibility = "public" })
        return "ok"
        "#,
    )
    .await;
    h.index().await;

    let outcome = h
        .run(
            r#"
        local hits = memory.search("David")
        if #hits == 0 then return "none" end
        hits[1]:append("Enjoys sailing", { by_agent = true, visibility = "public" })
        return "wrote"
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert!(result.contains("wrote"), "{result}");

    let read_back = h
        .run(r#"return memory.get("person/david"):details()"#)
        .await;
    let BlockOutcome::Committed { result } = read_back else {
        panic!("expected commit, got {read_back:?}");
    };
    assert!(
        result.contains("Enjoys sailing"),
        "the write landed: {result}"
    );
}

#[tokio::test]
async fn a_multi_word_query_naming_the_hit_writes_through() {
    // A multi-word query passes when one of its tokens names the hit: "Marcus Chen" carries the token
    // "marcus", which equals the person/marcus segment, so the write through the hit commits.
    let h = Harness::with_retrieval();
    h.run(
        r#"
        local marcus = memory.create(PERSON_MARCUS)
        marcus:append("Marcus Chen leads QA", { by_agent = true, visibility = "public" })
        return "ok"
        "#,
    )
    .await;
    h.index().await;

    let outcome = h
        .run(
            r#"
        local hits = memory.search("Marcus Chen")
        if #hits == 0 then return "none" end
        hits[1]:append("Based in Sydney", { by_agent = true, visibility = "public" })
        return "wrote"
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert!(result.contains("wrote"), "{result}");
}

#[tokio::test]
async fn a_stem_query_does_not_pass_the_guard() {
    // A stem is not proof of identity: searching "dav" and landing person/david does not license a
    // write, since "dav" is only a prefix of "david", never an exact token match.
    let h = Harness::with_retrieval();
    h.run(
        r#"
        local david = memory.create("person/david")
        david:append("Also goes by Dav", { by_agent = true, visibility = "public" })
        return "ok"
        "#,
    )
    .await;
    h.index().await;

    let outcome = h
        .run(
            r#"
        local hits = memory.search("dav")
        if #hits == 0 then return "none" end
        hits[1]:append("Runs the standup", { by_agent = true, visibility = "public" })
        return "wrote"
        "#,
        )
        .await;
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = outcome else {
        panic!("expected the guard to refuse a stem write, got {outcome:?}");
    };
    assert!(
        message.contains("search for \"dav\"") && message.contains("names person/david"),
        "a stem does not name the handle: {message}"
    );
}

#[tokio::test]
async fn links_create_gates_a_fuzzy_endpoint_but_allows_a_confirmed_one() {
    // A relationship recorded against the wrong referent is the same error :append guards, so a fuzzy
    // hit as either endpoint of links.create is refused — while a confirmed handle (from memory.get)
    // links cleanly.
    let h = Harness::with_retrieval();
    h.run(
        r#"
        links.register({ name = "knows", inverse = "known_by", from_card = "many", to_card = "many" })
        local david = memory.create("person/david")
        david:append("Introduced to Davina at the design sync", { by_agent = true, visibility = "public" })
        memory.create(TOPIC_PLAN, "The launch plan")
        return "ok"
        "#,
    )
    .await;
    h.index().await;

    // A fuzzy endpoint is refused.
    let refused = h
        .run(
            r#"
        local hits = memory.search("Davina")
        if #hits == 0 then return "none" end
        links.create(hits[1], "knows", memory.get(TOPIC_PLAN))
        return "linked"
        "#,
        )
        .await;
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = refused else {
        panic!("expected a refused fuzzy link, got {refused:?}");
    };
    assert!(
        message.contains("names person/david"),
        "the link guard names the mismatched endpoint: {message}"
    );

    // A confirmed endpoint links cleanly.
    let linked = h
        .run(
            r#"
        links.create(memory.get("person/david"), "knows", memory.get(TOPIC_PLAN))
        return "linked"
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = linked else {
        panic!("expected the confirmed link to commit, got {linked:?}");
    };
    assert!(result.contains("linked"), "{result}");
}

#[tokio::test]
async fn reads_through_a_fuzzy_hit_stay_free() {
    // The guard scopes to writes only: reading a fuzzy hit — its entries, its details, interpolating
    // it into a reply — is never gated, so recall off a hit works however far its name is from the query.
    let h = Harness::with_retrieval();
    h.run(
        r#"
        local david = memory.create("person/david")
        david:append("Introduced to Davina at the design sync", { by_agent = true, visibility = "public" })
        return "ok"
        "#,
    )
    .await;
    h.index().await;

    let outcome = h
        .run(
            r#"
        local hits = memory.search("Davina")
        if #hits == 0 then return "none" end
        local hit = hits[1]
        local es = hit:entries()
        local detail = hit:details()
        return `entries:{#es > 0} detail:{detail:find("design sync") ~= nil} interp:{hit.name}`
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected reads to commit, got {outcome:?}");
    };
    assert!(result.contains("entries:true"), "entries read: {result}");
    assert!(result.contains("detail:true"), "details read: {result}");
    assert!(
        result.contains("interp:person/david"),
        "interpolation reads the name: {result}"
    );
}

#[tokio::test]
async fn memory_list_results_are_never_gated() {
    // memory.list is exact-stem discovery — its handles are literal name matches, carrying no search
    // provenance — so a write through a list result is never gated.
    let h = Harness::with_retrieval();
    h.run(
        r#"
        local david = memory.create("person/david")
        david:append("Leads the platform team", { by_agent = true, visibility = "public" })
        return "ok"
        "#,
    )
    .await;

    let outcome = h
        .run(
            r#"
        local ms = memory.list("person/dav")
        if #ms == 0 then return "none" end
        ms[1]:append("Enjoys sailing", { by_agent = true, visibility = "public" })
        return "wrote"
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected a list write to commit, got {outcome:?}");
    };
    assert!(result.contains("wrote"), "{result}");
}

#[tokio::test]
async fn the_taught_confirmation_path_works_on_the_next_block() {
    // The block-boundary practice the taint guard teaches: the block that searched was composed before
    // its results were visible, so its taint refuses even a fetched-handle write to the surfaced memory
    // (the accepted cost — see `guard_search_taint`). The confirmation lands in the *next* block, which
    // was written after seeing the error and carries no taint, so memory.get the exact handle and write
    // through it — which commits.
    let h = Harness::with_retrieval();
    h.run(
        r#"
        local david = memory.create("person/david")
        david:append("Introduced to Davina at the design sync", { by_agent = true, visibility = "public" })
        return "ok"
        "#,
    )
    .await;
    h.index().await;

    // Same block: even a memory.get to the exact handle is refused, because this block's search for
    // "Davina" tainted person/david.
    let refused = h
        .run(
            r#"
        local hits = memory.search("Davina")
        memory.get("person/david"):append("Confirmed as the design lead", { by_agent = true, visibility = "public" })
        return "wrote"
        "#,
        )
        .await;
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = refused else {
        panic!("expected the same-block confirmation to be taint-refused, got {refused:?}");
    };
    assert!(
        message.contains("search in this block for \"Davina\"")
            && message.contains("surfaced person/david")
            && message.contains("in your next block"),
        "the refusal teaches the block boundary: {message}"
    );

    // Next block: fresh taint, so the confirmed write commits.
    let outcome = h
        .run(
            r#"
        memory.get("person/david"):append("Confirmed as the design lead", { by_agent = true, visibility = "public" })
        return "confirmed"
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected the next-block confirmed write to commit, got {outcome:?}");
    };
    assert!(result.contains("confirmed"), "{result}");

    let read_back = h
        .run(r#"return memory.get("person/david"):details()"#)
        .await;
    let BlockOutcome::Committed { result } = read_back else {
        panic!("expected commit, got {read_back:?}");
    };
    assert!(
        result.contains("Confirmed as the design lead"),
        "the confirmed write landed: {result}"
    );
}

#[tokio::test]
async fn a_launder_through_a_fetched_hit_name_is_refused() {
    // The observed launder the block-scoped taint closes: the whole if/else is composed before the
    // search runs, so an in-block branch on the result carries no judgement. The else-branch fetches the
    // mismatched hit by name (`memory.get(hits[1].name)`) — a provenance-free handle the fuzzy-write
    // guard never sees — and writes through it. The taint guard refuses it by target name regardless.
    let h = Harness::with_retrieval();
    h.run(
        r#"
        local david = memory.create("person/david")
        david:append("Introduced to Davina at the design sync", { by_agent = true, visibility = "public" })
        return "ok"
        "#,
    )
    .await;
    h.index().await;

    let outcome = h
        .run(
            r#"
        local hits = memory.search("Davina")
        if #hits == 0 then
            memory.create("person/davina"):append("Leads the design team", { by_agent = true, visibility = "public" })
        else
            local davina = memory.get(hits[1].name)
            davina:append("Leads the design team", { by_agent = true, visibility = "public" })
        end
        return "wrote"
        "#,
        )
        .await;
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = outcome else {
        panic!("expected the taint guard to refuse the laundered write, got {outcome:?}");
    };
    assert!(
        message.contains("search in this block for \"Davina\"")
            && message.contains("surfaced person/david"),
        "the error names the query and the tainted memory: {message}"
    );
    assert!(
        message.contains("candidate, not a match") && message.contains("in your next block"),
        "the error teaches the block boundary: {message}"
    );
    assert!(
        message.contains("memory.get(\"person/david\")")
            && message.contains("memory.create(\"person/davina\")"),
        "the error offers the next-block get/create decision: {message}"
    );

    // Nothing landed on David.
    let read_back = h
        .run(r#"return memory.get("person/david"):details()"#)
        .await;
    let BlockOutcome::Committed { result } = read_back else {
        panic!("expected commit, got {read_back:?}");
    };
    assert!(
        !result.contains("Leads the design team"),
        "the laundered write must not have committed: {result}"
    );
}

#[tokio::test]
async fn a_rename_through_a_mismatched_hit_is_refused() {
    // Renaming rewrites identity, so it is guarded like the content writers — and an ungated rename
    // would launder the taint outright: rename the mismatched hit to the name you meant, and a write
    // through the new name would no longer look tainted.
    let h = Harness::with_retrieval();
    h.run(
        r#"
        local david = memory.create("person/david")
        david:append("Introduced to Davina at the design sync", { by_agent = true, visibility = "public" })
        return "ok"
        "#,
    )
    .await;
    h.index().await;

    let outcome = h
        .run(
            r#"
        local hits = memory.search("Davina")
        hits[1]:rename("person/davina")
        return "renamed"
        "#,
        )
        .await;
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = outcome else {
        panic!("expected the guard to refuse the rename, got {outcome:?}");
    };
    assert!(
        message.contains("candidate, not a match"),
        "the refusal is the fuzzy-write guard's: {message}"
    );

    // The taint variant: fetching the mismatched hit's name and renaming through the clean handle in
    // the same block is refused by target name.
    let outcome = h
        .run(
            r#"
        local hits = memory.search("Davina")
        local h2 = memory.get(hits[1].name)
        h2:rename("person/davina")
        return "renamed"
        "#,
        )
        .await;
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = outcome else {
        panic!("expected the taint guard to refuse the laundered rename, got {outcome:?}");
    };
    assert!(
        message.contains("in your next block"),
        "the refusal teaches the block boundary: {message}"
    );

    // A next-block rename through a confirmed handle stays free.
    let renamed = h
        .run(r#"memory.get("person/david"):rename("person/dave_2"); return "ok""#)
        .await;
    assert!(
        matches!(renamed, BlockOutcome::Committed { .. }),
        "a cross-block confirmed rename passes: {renamed:?}"
    );
}

#[tokio::test]
async fn links_create_to_a_tainted_name_is_refused_in_block() {
    // The taint gates a `links.create` endpoint too: a relationship recorded against a memory a
    // mismatched search surfaced is the same wrong-referent error a content write is. The endpoint is
    // fetched by the hit's name, so only the taint guard — not the hit-handle guard — catches it.
    let h = Harness::with_retrieval();
    h.run(
        r#"
        local david = memory.create("person/david")
        david:append("Introduced to Davina at the design sync", { by_agent = true, visibility = "public" })
        memory.create("person/team")
        return "ok"
        "#,
    )
    .await;
    h.index().await;

    let outcome = h
        .run(
            r#"
        local hits = memory.search("Davina")
        local davina = memory.get(hits[1].name)
        links.create(davina, "knows", memory.get("person/team"))
        return "linked"
        "#,
        )
        .await;
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = outcome else {
        panic!("expected the taint guard to refuse the link, got {outcome:?}");
    };
    assert!(
        message.contains("search in this block for \"Davina\"")
            && message.contains("surfaced person/david"),
        "the link refusal names the query and the tainted memory: {message}"
    );
}

#[tokio::test]
async fn a_same_block_write_to_a_named_memory_is_unaffected() {
    // The taint records only *mismatched* hits: a search whose token names its hit taints nothing, so a
    // same-block write to that memory — through any handle — is free. "David" names person/david, so the
    // fetched-handle append in the same block as the search commits.
    let h = Harness::with_retrieval();
    h.run(
        r#"
        local david = memory.create("person/david")
        david:append("Leads the platform team", { by_agent = true, visibility = "public" })
        return "ok"
        "#,
    )
    .await;
    h.index().await;

    let outcome = h
        .run(
            r#"
        local hits = memory.search("David")
        if #hits == 0 then return "none" end
        memory.get("person/david"):append("Enjoys sailing", { by_agent = true, visibility = "public" })
        return "wrote"
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected the named-memory write to commit, got {outcome:?}");
    };
    assert!(result.contains("wrote"), "{result}");

    let read_back = h
        .run(r#"return memory.get("person/david"):details()"#)
        .await;
    let BlockOutcome::Committed { result } = read_back else {
        panic!("expected commit, got {read_back:?}");
    };
    assert!(
        result.contains("Enjoys sailing"),
        "the write landed: {result}"
    );
}

#[tokio::test]
async fn taint_does_not_leak_across_blocks() {
    // The taint dies with the block that minted it: a mismatched search in one block, with no write,
    // leaves the next block free to write to the surfaced memory. The cross-block asymmetry is the whole
    // point — the retry block was composed after the results were visible.
    let h = Harness::with_retrieval();
    h.run(
        r#"
        local david = memory.create("person/david")
        david:append("Introduced to Davina at the design sync", { by_agent = true, visibility = "public" })
        return "ok"
        "#,
    )
    .await;
    h.index().await;

    // Block one: a mismatched search taints person/david, but the block writes nothing.
    let searched = h
        .run(
            r#"
        local hits = memory.search("Davina")
        return #hits
        "#,
        )
        .await;
    assert!(matches!(searched, BlockOutcome::Committed { .. }));

    // Block two: no search ran here, so no taint — the write to person/david commits.
    let outcome = h
        .run(
            r#"
        memory.get("person/david"):append("Leads the design team", { by_agent = true, visibility = "public" })
        return "wrote"
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected the next-block write to commit, got {outcome:?}");
    };
    assert!(result.contains("wrote"), "{result}");
}

#[tokio::test]
async fn the_guard_error_is_catchable_and_the_block_rolls_back() {
    // The guard error is an ordinary Lua runtime error — pcall catches it — and an uncaught one
    // terminates the block, rolling back every other effect it buffered (block transactionality).
    let h = Harness::with_retrieval();
    h.run(
        r#"
        local david = memory.create("person/david")
        david:append("Introduced to Davina at the design sync", { by_agent = true, visibility = "public" })
        return "ok"
        "#,
    )
    .await;
    h.index().await;

    // Catchable: pcall returns false with the teachable message.
    let caught = h
        .run(
            r#"
        local hits = memory.search("Davina")
        local ok, err = pcall(function()
            hits[1]:append("Leads the design team", { by_agent = true, visibility = "public" })
        end)
        return `ok:{ok} caught:{tostring(err):find("candidate, not a match") ~= nil}`
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = caught else {
        panic!("expected the pcall block to commit, got {caught:?}");
    };
    assert!(
        result.contains("ok:false"),
        "pcall caught the error: {result}"
    );
    assert!(
        result.contains("caught:true"),
        "the message is the guard's: {result}"
    );

    // Uncaught: an earlier write in the same block rolls back when the guard terminates it.
    let terminated = h
        .run(
            r#"
        memory.create("topic/scratch", "should roll back")
        local hits = memory.search("Davina")
        hits[1]:append("Leads the design team", { by_agent = true, visibility = "public" })
        return "unreached"
        "#,
        )
        .await;
    assert!(
        matches!(
            terminated,
            BlockOutcome::Terminated(TerminalCause::Error(_))
        ),
        "the uncaught guard error terminates the block: {terminated:?}"
    );
    let scratch = h
        .run(r#"return tostring(memory.get("topic/scratch"))"#)
        .await;
    let BlockOutcome::Committed { result } = scratch else {
        panic!("expected commit, got {scratch:?}");
    };
    assert_eq!(
        result, "nil",
        "the terminated block's create rolled back: {result}"
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
