//! `mem:find_entry(text)` — locating a live entry by a phrase to correct it, without the text-scan
//! loop that misses on casing and paraphrase. The match is case-insensitive and diacritic-folded,
//! reads exactly the set `mem:entries` returns (including this block's pending appends), returns a lone
//! match's entry object or nil, and surfaces ambiguity teachably rather than taking the first hit.

use crate::{BlockOutcome, Harness, Namespace, TerminalCause};

#[tokio::test]
async fn find_entry_matches_case_insensitively() {
    // The needle folds case, so a lower-case phrase finds an entry recorded with different casing —
    // the slip a `e.text:find(...)` loop makes when it compares raw.
    let h = Harness::new();
    let outcome = h
        .run(
            r#"
        local dave = memory.create(PERSON_DAVE)
        dave:append("Leads the Volcano Project", { visibility = "public" })
        local e = dave:find_entry("leads the volcano project")
        if e == nil then return "nil" end
        return `found: {e}`
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert!(
        result.contains("Leads the Volcano Project"),
        "the found entry should render as its own text, got: {result}"
    );
}

#[tokio::test]
async fn find_entry_folds_diacritics_both_ways() {
    // The needle and the entry text fold to the same unaccented base, so an accented needle finds an
    // unaccented entry and the reverse — the fold the fuzzy-write guard already uses.
    let h = Harness::new();
    let outcome = h
        .run(
            r#"
        local dave = memory.create(PERSON_DAVE)
        dave:append("Lives in malmo now", { visibility = "public" })
        local e = dave:find_entry("Malmö")
        if e == nil then return "nil" end
        return `found: {e}`
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert!(
        result.contains("Lives in malmo now"),
        "an accented needle should find the unaccented entry, got: {result}"
    );

    // And the reverse: an unaccented needle finds an accented entry.
    let h = Harness::new();
    let outcome = h
        .run(
            r#"
        local dave = memory.create(PERSON_DAVE)
        dave:append("Lives in Malmö now", { visibility = "public" })
        local e = dave:find_entry("malmo")
        if e == nil then return "nil" end
        return `found: {e}`
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert!(
        result.contains("Malmö"),
        "an unaccented needle should find the accented entry, got: {result}"
    );
}

#[tokio::test]
async fn find_entry_returns_nil_when_nothing_matches() {
    // No match is nil, not an error — the caller branches on it (create instead of correct).
    let h = Harness::new();
    let outcome = h
        .run(
            r#"
        local dave = memory.create(PERSON_DAVE)
        dave:append("Leads the volcano project", { visibility = "public" })
        local e = dave:find_entry("scuba diving")
        if e == nil then return "no match" end
        return "unexpected match"
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert!(
        result.starts_with("no match"),
        "an absent phrase should return nil, got: {result}"
    );
}

#[tokio::test]
async fn find_entry_surfaces_ambiguity_with_candidates() {
    // Two entries share the needle, so returning the first silently would be the correct-the-wrong-
    // entry hazard. The ambiguity is a teachable error listing each candidate, pointing at a longer
    // phrase or the id.
    let h = Harness::new();
    let outcome = h
        .run(
            r#"
        local dave = memory.create(PERSON_DAVE)
        dave:append("Works on the volcano project", { visibility = "public" })
        dave:append("Loves the volcano documentary", { visibility = "public" })
        dave:find_entry("volcano")
        "#,
        )
        .await;
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = outcome else {
        panic!("expected a teachable error, got {outcome:?}");
    };
    assert!(
        message.contains("matches more than one entry"),
        "the ambiguity should be named, got: {message}"
    );
    assert!(
        message.contains("use a longer") && message.contains("id"),
        "it should point at a longer phrase or the id, got: {message}"
    );
    assert!(
        message.contains("volcano project") && message.contains("volcano documentary"),
        "each candidate's snippet should be listed, got: {message}"
    );
}

#[tokio::test]
async fn find_entry_rejects_an_empty_needle() {
    // A whitespace needle folds to empty — a match-anything scan, not a find — so it is refused before
    // it reads.
    let h = Harness::new();
    let outcome = h
        .run(
            r#"
        local dave = memory.create(PERSON_DAVE)
        dave:append("Leads the volcano project", { visibility = "public" })
        dave:find_entry("   ")
        "#,
        )
        .await;
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = outcome else {
        panic!("expected a teachable error, got {outcome:?}");
    };
    assert!(
        message.contains("empty needle") || message.contains("needs some text"),
        "an empty needle should be a teachable error, got: {message}"
    );
}

#[tokio::test]
async fn find_entry_sees_a_pending_append_in_the_same_block() {
    // find_entry mirrors mem:entries: it reads the class's live entries plus this block's pending
    // appends, so an entry appended earlier in the same block is findable (read-your-writes) before it
    // commits.
    let h = Harness::new();
    let outcome = h
        .run(
            r#"
        local dave = memory.create(PERSON_DAVE)
        dave:append("Just joined the climbing gym", { visibility = "public" })
        local e = dave:find_entry("climbing gym")
        if e == nil then return "nil" end
        return `found: {e}`
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert!(
        result.contains("Just joined the climbing gym"),
        "a pending append should be findable in the same block, got: {result}"
    );
}

#[tokio::test]
async fn find_entry_drives_a_one_block_correction() {
    // The idiom find_entry completes: a fact filed on the wrong person, corrected in ONE block —
    // find_entry locates it, retract withdraws it, and a fresh append re-asserts it on the right
    // person, with no text-scan loop.
    let h = Harness::new();
    let seeded = h
        .run(
            r#"
        local erin = memory.create(PERSON_ERIN)
        erin:append("Leads the volcano project", { visibility = "public" })
        return "seeded"
        "#,
        )
        .await;
    let BlockOutcome::Committed { .. } = seeded else {
        panic!("expected the seed to commit, got {seeded:?}");
    };

    let outcome = h
        .run(
            r#"
        local erin = memory.get(PERSON_ERIN)
        local dave = memory.get_or_create(PERSON_DAVE)
        local wrong = erin:find_entry("leads the volcano project")
        erin:retract(wrong, "filed on the wrong person")
        dave:append("Leads the volcano project", { visibility = "public", by_agent = true })
        return "corrected"
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected the correction to commit, got {outcome:?}");
    };
    assert!(result.starts_with("corrected"), "got: {result}");

    // The fact moved: erin holds nothing live, dave holds the re-asserted entry.
    let erin = h
        .engine
        .graph
        .lock()
        .memory_by_name(Namespace::Person.with_name("erin"))
        .unwrap()
        .unwrap();
    assert!(
        h.engine
            .graph
            .lock()
            .entries_local(erin.id)
            .unwrap()
            .is_empty(),
        "the mis-filed fact should be retracted off erin"
    );
    let dave = h
        .engine
        .graph
        .lock()
        .memory_by_name(Namespace::Person.with_name("dave"))
        .unwrap()
        .unwrap();
    let dave_live: Vec<String> = h
        .engine
        .graph
        .lock()
        .entries_local(dave.id)
        .unwrap()
        .into_iter()
        .map(|e| e.text)
        .collect();
    assert_eq!(dave_live, ["Leads the volcano project"]);
}
