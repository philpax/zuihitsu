//! The fuzzy-write guard: a content write (or a `links.create` endpoint) through a `memory.search`
//! hit whose query does not name the handle it landed on is refused before it commits, while an
//! exact-, multi-word-, or list-named target writes through cleanly. Reads through a fuzzy hit stay
//! free, and the refusal is an ordinary catchable Lua error that rolls the block back.

use super::*;

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
