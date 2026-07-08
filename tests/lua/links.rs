use super::*;

#[tokio::test]
async fn link_with_an_unregistered_relation_is_a_teachable_error() {
    let h = Harness::new();
    h.run(r#"memory.create(TOPIC_A)"#).await;
    // No such relation is registered: the block fails with a teachable error and commits nothing.
    let outcome = h
        .run(r#"memory.get(TOPIC_A):link("bogus_rel", memory.get(TOPIC_A))"#)
        .await;
    match outcome {
        BlockOutcome::Terminated(TerminalCause::Error(message)) => {
            assert!(
                message.contains("unknown relation"),
                "message was: {message}"
            );
        }
        other => panic!("expected a teachable error, got {other:?}"),
    }
}

#[tokio::test]
async fn link_and_unlink_resolve_a_name_string_target() {
    // A name string in place of a handle is looked up, not rejected with a type error that would roll
    // the whole block back — the cascade that silently dropped a co-located private write (#43). This
    // block links via a string *and* appends a confidence in one go; both must survive together. Unlink
    // shares the same resolution seam, so a name string clears the edge too.
    let h = Harness::new();
    // The Harness skips genesis, so register the `knows` relation the link instantiates.
    h.engine
        .store
        .lock()
        .append(
            h.clock.now(),
            vec![EventPayload::LinkTypeRegistered {
                name: RelationName::Knows,
                inverse: RelationName::Knows,
                from_card: Cardinality::Many,
                to_card: Cardinality::Many,
                symmetric: true,
                reflexive: false,
                description: String::new(),
            }],
        )
        .unwrap();
    h.engine
        .graph
        .lock()
        .materialize_from(h.engine.store.lock().as_ref())
        .unwrap();
    h.run(r#"memory.create(PERSON_DAVE)"#).await;
    h.run(r#"memory.create(PERSON_ERIN)"#).await;

    // PERSON_ERIN substitutes to a bare name string, not a handle, so this exercises the string-target
    // path; the private append in the same block proves it does not error-and-roll-back.
    let outcome = h
        .run(
            r#"local dave = memory.get(PERSON_DAVE)
               dave:link("knows", PERSON_ERIN, { visibility = "public" })
               dave:append("a quiet aside", { visibility = "private" })"#,
        )
        .await;
    assert!(
        matches!(outcome, BlockOutcome::Committed { .. }),
        "a string-target link must commit (with its co-located write), got {outcome:?}"
    );

    // The string target resolved to a real edge — an outgoing `knows` link now exists.
    let BlockOutcome::Committed { result } = h
        .run(r#"return memory.get(PERSON_DAVE):outgoing("knows")"#)
        .await
    else {
        panic!("expected a committed read");
    };
    assert!(
        !result.trim().is_empty(),
        "a knows edge should exist, got empty: {result:?}"
    );

    // Unlink through the same seam: a name string clears the edge just as it made it.
    let unlink_outcome = h
        .run(r#"memory.get(PERSON_DAVE):unlink("knows", PERSON_ERIN)"#)
        .await;
    assert!(
        matches!(unlink_outcome, BlockOutcome::Committed { .. }),
        "a string-target unlink must commit, got {unlink_outcome:?}"
    );
    let BlockOutcome::Committed { result } = h
        .run(r#"return memory.get(PERSON_DAVE):outgoing("knows")"#)
        .await
    else {
        panic!("expected a committed read");
    };
    assert!(
        !result.contains("erin"),
        "the knows edge should be gone after unlinking by name, got: {result:?}"
    );
}

#[tokio::test]
async fn link_to_an_unknown_name_teaches_creation() {
    // A name string that names no memory is a teachable miss — it says the name is unknown and points at
    // creating it or checking the casing, rather than lecturing about handles, so the agent's fix is to
    // create the memory (or correct a typo), not to reach for a handle it does not have.
    let h = Harness::new();
    h.run(r#"memory.create(PERSON_DAVE)"#).await;
    let outcome = h
        .run(r#"memory.get(PERSON_DAVE):link("knows", "person/nobody", { visibility = "public" })"#)
        .await;
    match outcome {
        BlockOutcome::Terminated(TerminalCause::Error(message)) => {
            assert!(
                message.contains("no memory named \"person/nobody\"")
                    && message.contains("create it first")
                    && message.contains("casing"),
                "the unknown name should teach creation/casing, got: {message}"
            );
        }
        other => panic!("expected a teachable unknown-name error, got {other:?}"),
    }
}

#[tokio::test]
async fn a_memory_handle_renders_its_link_neighborhood() {
    // A topic hub prints its links line, so a recall that fetches the hub sees the spokes its
    // decisions live on — the linked events — rather than reading only the hub's own entries and
    // dropping a fact that sits one link away (the hub-and-spoke recall gap). The links are committed
    // in one block, then the hub is fetched in the next (block.links reflects committed state).
    let h = Harness::new();
    h.run(
        r#"
        links.register({ name = "part_of", inverse = "contains", from_card = "many", to_card = "many" })
        local topic = memory.create(TOPIC_MIGRATION, "The billing migration")
        local ship = memory.create(EVENT_LAUNCH, "Ship the migration")
        ship:link("part_of", topic)
        "#,
    )
    .await;

    let BlockOutcome::Committed { result } = h.run(r#"return memory.get(TOPIC_MIGRATION)"#).await
    else {
        panic!("expected a committed read");
    };
    assert!(
        result.contains("links:"),
        "the handle should render a links line, got: {result}"
    );
    assert!(
        result.contains("part_of")
            && result.contains(MemoryName::from(Namespace::Event.with_name("launch")).as_str()),
        "the links line should name the relation and the linked event, got: {result}"
    );
}

#[tokio::test]
async fn a_dated_link_target_shows_its_occurrence_on_the_handle() {
    // A dated spoke carries its date onto the hub's links line (the same `[when …]` phrasing a search
    // hit uses), so relaying the recap from the handle keeps the *when* without a separate read.
    let h = Harness::new();
    h.run(
        r#"
        links.register({ name = "part_of", inverse = "contains", from_card = "many", to_card = "many" })
        local topic = memory.create(TOPIC_MIGRATION, "The billing migration")
        local ship = memory.create(EVENT_LAUNCH)
        ship:append("Ship it", { visibility = "public", occurred_at = { day = "2026-08-01" } })
        ship:link("part_of", topic)
        "#,
    )
    .await;

    let BlockOutcome::Committed { result } = h.run(r#"return memory.get(TOPIC_MIGRATION)"#).await
    else {
        panic!("expected a committed read");
    };
    assert!(
        result.contains("[when 2026-08-01]"),
        "the dated spoke should show its occurrence on the links line, got: {result}"
    );
}

#[tokio::test]
async fn the_neighborhood_line_caps_and_notes_the_remainder() {
    // A busy hub does not flood the transcript: the links line shows the first several and elides the
    // rest with a `(+N more)` note. Nine events linked to the topic exceeds the cap of eight.
    let h = Harness::new();
    h.run(
        r#"
        links.register({ name = "part_of", inverse = "contains", from_card = "many", to_card = "many" })
        local topic = memory.create(TOPIC_MIGRATION, "The billing migration")
        for i = 1, 9 do
            local ev = memory.create("event/spoke-" .. i)
            ev:link("part_of", topic)
        end
        "#,
    )
    .await;

    let BlockOutcome::Committed { result } = h.run(r#"return memory.get(TOPIC_MIGRATION)"#).await
    else {
        panic!("expected a committed read");
    };
    assert!(
        result.contains("(+1 more)"),
        "the links line should cap and note the elided remainder, got: {result}"
    );
}

#[tokio::test]
async fn link_readers_traverse_the_merged_identity() {
    // The link readers (spec §Lua API → link readers) auto-traverse the same_as class: an edge on one
    // stub surfaces when read through any member, oriented against the identity, with the same_as
    // plumbing itself excluded.
    let h = Harness::new();
    // The Harness skips genesis, so register the relations the test links under.
    h.engine
        .store
        .lock()
        .append(
            h.clock.now(),
            vec![
                EventPayload::LinkTypeRegistered {
                    name: RelationName::SameAs,
                    inverse: RelationName::SameAs,
                    from_card: Cardinality::Many,
                    to_card: Cardinality::Many,
                    symmetric: true,
                    reflexive: false,
                    description: String::new(),
                },
                EventPayload::LinkTypeRegistered {
                    name: RelationName::new("mentor_of"),
                    inverse: RelationName::new("mentored_by"),
                    from_card: Cardinality::Many,
                    to_card: Cardinality::Many,
                    symmetric: false,
                    reflexive: false,
                    description: String::new(),
                },
                EventPayload::LinkTypeRegistered {
                    name: RelationName::new("works_at"),
                    inverse: RelationName::new("employs"),
                    from_card: Cardinality::Many,
                    to_card: Cardinality::One,
                    symmetric: false,
                    reflexive: false,
                    description: String::new(),
                },
            ],
        )
        .unwrap();
    h.engine
        .graph
        .lock()
        .materialize_from(h.engine.store.lock().as_ref())
        .unwrap();

    // A two-stub Dave identity, plus the people and the company it links to.
    for name in [
        MemoryName::from(Namespace::Person.with_name("dave")).as_str(),
        MemoryName::from(Namespace::Person.with_name("dave@discord")).as_str(),
        MemoryName::from(Namespace::Person.with_name("erin")).as_str(),
        MemoryName::from(Namespace::Person.with_name("frank")).as_str(),
        "company/hooli",
    ] {
        h.run(&format!("memory.create({name:?})")).await;
    }

    // Merge the two Dave stubs — operator-only.
    let operator = BlockContext {
        teller: Teller::Agent,
        authority: Authority::Operator,
        turn_id: TurnId::generate(),
        block_timeout: TEST_BLOCK_TIMEOUT,
        max_block_attempts: TEST_MAX_BLOCK_ATTEMPTS,
        present_set: Vec::new(),
        dry_run: false,
    };
    h.session
        .execute(
            &h.engine,
            &operator,
            &common::prepare_script(
                r#"memory.get(PERSON_DAVE):link("same_as", memory.get(PERSON_DAVE_AT_DISCORD))"#,
            ),
        )
        .await
        .unwrap();

    // Links spread across the two stubs: one mentors Erin, Frank mentors the other, and the other
    // works at Hooli — so a class-blind read of the primary stub would miss two of the three.
    h.run(r#"memory.get(PERSON_DAVE):link("mentor_of", memory.get(PERSON_ERIN), { visibility = "public" })"#)
        .await;
    h.run(r#"memory.get(PERSON_FRANK):link("mentor_of", memory.get(PERSON_DAVE_AT_DISCORD), { visibility = "public" })"#)
        .await;
    h.run(r#"memory.get(PERSON_DAVE_AT_DISCORD):link("works_at", memory.get("company/hooli"))"#)
        .await;

    // outgoing: who Dave mentors — Erin, reached through the merged identity though queried via the
    // primary stub. A single edge, so the list renders as the one readable line.
    let BlockOutcome::Committed { result } = h
        .run(r#"return memory.get(PERSON_DAVE):outgoing("mentor_of")"#)
        .await
    else {
        panic!("expected commit");
    };
    assert_eq!(
        result,
        format!(
            "mentor_of → {}",
            MemoryName::from(Namespace::Person.with_name("erin")).as_str()
        )
    );

    // incoming: who mentors Dave — Frank, whose edge lands on the *other* stub, surfaced by traversal.
    let BlockOutcome::Committed { result } = h
        .run(r#"return memory.get(PERSON_DAVE):incoming("mentor_of")"#)
        .await
    else {
        panic!("expected commit");
    };
    assert_eq!(
        result,
        format!(
            "mentor_of ← {}",
            MemoryName::from(Namespace::Person.with_name("frank")).as_str()
        )
    );

    // links(): the whole relationship set across the identity — both mentor_of edges and works_at —
    // with the same_as edge holding the identity together excluded as internal plumbing.
    let BlockOutcome::Committed { result } =
        h.run(r#"return memory.get(PERSON_DAVE):links()"#).await
    else {
        panic!("expected commit");
    };
    assert!(
        result.contains(&format!(
            "mentor_of → {}",
            MemoryName::from(Namespace::Person.with_name("erin")).as_str()
        )),
        "{result}"
    );
    assert!(
        result.contains(&format!(
            "mentor_of ← {}",
            MemoryName::from(Namespace::Person.with_name("frank")).as_str()
        )),
        "{result}"
    );
    assert!(result.contains("works_at → company/hooli"), "{result}");
    assert!(
        !result.contains("same_as"),
        "the same_as plumbing must not surface as a relationship: {result}"
    );

    // A script branches on the structured fields, not only the rendered line — including `told_by`,
    // the teller behind the link (here the agent itself, "you", since these were agent-authored).
    let BlockOutcome::Committed { result } = h
        .run(
            r#"
        local out = memory.get(PERSON_DAVE):outgoing("mentor_of")
        return out[1].name .. " / " .. out[1].direction .. " / " .. out[1].source
            .. " / " .. out[1].told_by
        "#,
        )
        .await
    else {
        panic!("expected commit");
    };
    assert_eq!(
        result,
        format!(
            "{} / outgoing / agent / you",
            MemoryName::from(Namespace::Person.with_name("erin")).as_str()
        )
    );
}
