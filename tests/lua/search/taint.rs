//! The block-scoped taint guard: a mismatched `memory.search` this block surfaced taints the target
//! name, so a write reaching it however indirectly — a fetched-handle append, a laundered
//! `memory.get(hits[1].name)`, a `links.create` endpoint — is refused within the block, while a
//! same-block write to a *named* memory is untouched and the taint dies with the block.

use super::*;

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
