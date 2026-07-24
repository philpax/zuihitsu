use crate::{BlockOutcome, Harness, MemoryName, Namespace, TerminalCause};

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
// window, the `ref` field round-tripping through `message_refs::scan`, and the feature-off nil-call.
