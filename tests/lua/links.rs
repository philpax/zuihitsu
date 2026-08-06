use crate::{
    Authority, BlockContext, BlockOutcome, Cardinality, Clock, EventPayload, EventSource, Harness,
    MemoryName, Namespace, RelationName, TEST_BLOCK_TIMEOUT, TEST_MAX_BLOCK_ATTEMPTS,
    TEST_MAX_ENTRY_CHARS, Teller, TerminalCause, TurnId, common,
};

/// Register the symmetric `knows` relation and materialize it. The [`Harness`] skips genesis, so a
/// test whose link instantiates a seed relation registers it first.
async fn register_knows(h: &Harness) {
    h.engine
        .store
        .lock()
        .append(
            h.clock.now(),
            EventSource::Agent,
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
}

#[tokio::test]
async fn link_with_an_unregistered_relation_is_a_teachable_error() {
    let h = Harness::new();
    h.run(r#"memory.create(TOPIC_A)"#).await;
    // No such relation is registered: the block fails with a teachable error and commits nothing.
    let outcome = h
        .run(r#"links.create(memory.get(TOPIC_A), "bogus_rel", memory.get(TOPIC_A))"#)
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
    register_knows(&h).await;
    h.run(r#"memory.create(PERSON_DAVE)"#).await;
    h.run(r#"memory.create(PERSON_ERIN)"#).await;

    // PERSON_ERIN substitutes to a bare name string, not a handle, so this exercises the string-target
    // path; the private append in the same block proves it does not error-and-roll-back.
    let outcome = h
        .run(
            r#"local dave = memory.get(PERSON_DAVE)
               links.create(dave, "knows", PERSON_ERIN, { visibility = "public" })
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
        .run(r#"links.remove(memory.get(PERSON_DAVE), "knows", PERSON_ERIN)"#)
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
async fn link_to_a_free_name_mints_the_endpoint() {
    // Naming the far end of a relationship before anything is known about it is an ordinary shape, so a
    // name string that shadows no existing handle is created bare rather than failing the block — which
    // would roll back whatever else the block wrote alongside it.
    let h = Harness::new();
    register_knows(&h).await;
    h.run(r#"memory.create(PERSON_DAVE)"#).await;
    let outcome = h
        .run(r#"links.create(memory.get(PERSON_DAVE), "knows", PERSON_NOBODY, { visibility = "public" })"#)
        .await;
    assert!(
        matches!(outcome, BlockOutcome::Committed { .. }),
        "a free endpoint name should mint and commit, got {outcome:?}"
    );
    let BlockOutcome::Committed { result } = h
        .run(r#"return memory.get(PERSON_DAVE):outgoing("knows")"#)
        .await
    else {
        panic!("expected a committed read");
    };
    assert!(
        result.contains("nobody"),
        "the minted endpoint should carry the edge, got: {result}"
    );
}

#[tokio::test]
async fn an_uninterpolated_endpoint_name_is_never_minted() {
    // A plain quoted string does not interpolate, so `"person/{name}"` reaches here with its braces
    // intact. Minting it would put the agent's own slip in the log permanently, under a handle nothing
    // will ever resolve — the placeholder guard every other minting path runs must hold here too.
    let h = Harness::new();
    register_knows(&h).await;
    h.run(r#"memory.create(PERSON_DAVE)"#).await;
    let outcome = h
        .run(r#"links.create(memory.get(PERSON_DAVE), "knows", "person/{name}", { visibility = "public" })"#)
        .await;
    assert!(
        matches!(outcome, BlockOutcome::Terminated(TerminalCause::Error(_))),
        "an uninterpolated endpoint must not commit, got {outcome:?}"
    );
    let BlockOutcome::Committed { result } = h.run(r#"return memory.list("person/")"#).await else {
        panic!("expected a committed read");
    };
    assert!(
        !result.contains("{name}"),
        "no memory should have been minted for the placeholder, got: {result}"
    );
}

#[tokio::test]
async fn unlinking_an_unknown_name_does_not_mint_it() {
    // Minting is `links.create`'s convenience only. An unlink naming a memory that does not exist is a
    // slip — creating the memory on the way to disconnecting from it is the opposite of the intent.
    let h = Harness::new();
    register_knows(&h).await;
    h.run(r#"memory.create(PERSON_DAVE)"#).await;
    let outcome = h
        .run(r#"links.remove(memory.get(PERSON_DAVE), "knows", PERSON_NOBODY)"#)
        .await;
    assert!(
        matches!(outcome, BlockOutcome::Terminated(TerminalCause::Error(_))),
        "an unlink against an unknown name must not commit, got {outcome:?}"
    );
    let BlockOutcome::Committed { result } = h.run(r#"return memory.list("person/")"#).await else {
        panic!("expected a committed read");
    };
    assert!(
        !result.contains("nobody"),
        "an unlink must not mint its target, got: {result}"
    );
}

#[tokio::test]
async fn a_free_form_endpoint_name_is_not_minted() {
    // The near-match guard is scoped to a namespace, so it is blind to a name carrying no recognised
    // prefix — nothing would catch `project/atlus` against an existing `project/atlas`. Minting is
    // therefore limited to the namespaces the guard can see, and a free-form name keeps the old error.
    let h = Harness::new();
    register_knows(&h).await;
    h.run(r#"memory.create(PERSON_DAVE)"#).await;
    let outcome = h
        .run(r#"links.create(memory.get(PERSON_DAVE), "knows", "project/atlas", { visibility = "public" })"#)
        .await;
    match outcome {
        BlockOutcome::Terminated(TerminalCause::Error(message)) => {
            assert!(
                message.contains("no memory named \"project/atlas\""),
                "a free-form endpoint should stay a teachable miss, got: {message}"
            );
        }
        other => panic!("expected a teachable unknown-name error, got {other:?}"),
    }
}

#[tokio::test]
async fn a_machinery_owned_endpoint_name_is_not_minted() {
    // A context/ memory is minted by the session machinery alongside a real room, and person/operator
    // by the imprint. Creating either here names something that does not exist, or squats a handle a
    // later imprint binds.
    let h = Harness::new();
    register_knows(&h).await;
    h.run(r#"memory.create(PERSON_DAVE)"#).await;
    for endpoint in [r#""context/chat:ghost-room""#, r#"PERSON_OPERATOR"#] {
        let outcome = h
            .run(&format!(
                r#"links.create(memory.get(PERSON_DAVE), "knows", {endpoint}, {{ visibility = "public" }})"#
            ))
            .await;
        assert!(
            matches!(outcome, BlockOutcome::Terminated(TerminalCause::Error(_))),
            "{endpoint} must not be minted, got {outcome:?}"
        );
    }
}

#[tokio::test]
async fn link_to_a_near_matching_name_teaches_the_neighbour() {
    // The one case minting is refused: a name a hair off an existing handle would split that subject's
    // facts across two memories, which no later read can see and no create can undo. The error lists the
    // neighbours so the fix is to pick the handle meant, not to invent a distinguishing name.
    let h = Harness::new();
    register_knows(&h).await;
    h.run(r#"memory.create(PERSON_DAVE)"#).await;
    h.run(r#"memory.create(PERSON_ERIN)"#).await;
    let outcome = h
        .run(r#"links.create(memory.get(PERSON_DAVE), "knows", "person/erim", { visibility = "public" })"#)
        .await;
    match outcome {
        BlockOutcome::Terminated(TerminalCause::Error(message)) => {
            assert!(
                message.contains("no memory named \"person/erim\"")
                    && message.contains("person/erin"),
                "a near-matching endpoint should name the neighbour, got: {message}"
            );
        }
        other => panic!("expected a teachable near-match error, got {other:?}"),
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
        links.create(ship, "part_of", topic)
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
        links.create(ship, "part_of", topic)
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
            links.create(ev, "part_of", topic)
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
            EventSource::Agent,
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
        MemoryName::from(Namespace::Person.with_name("dave@chat")).as_str(),
        MemoryName::from(Namespace::Person.with_name("erin")).as_str(),
        MemoryName::from(Namespace::Person.with_name("frank")).as_str(),
        "org/hooli",
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
        max_entry_chars: TEST_MAX_ENTRY_CHARS,
        present_set: Vec::new(),
        dry_run: false,
    };
    h.session
        .execute(
            &h.engine,
            &operator,
            &common::prepare_script(
                r#"links.create(memory.get(PERSON_DAVE), "same_as", memory.get(PERSON_DAVE_AT_CHAT))"#,
            ),
        )
        .await
        .unwrap();

    // Links spread across the two stubs: one mentors Erin, Frank mentors the other, and the other
    // works at Hooli — so a class-blind read of the primary stub would miss two of the three.
    h.run(r#"links.create(memory.get(PERSON_DAVE), "mentor_of", memory.get(PERSON_ERIN), { visibility = "public" })"#)
        .await;
    h.run(r#"links.create(memory.get(PERSON_FRANK), "mentor_of", memory.get(PERSON_DAVE_AT_CHAT), { visibility = "public" })"#)
        .await;
    h.run(r#"links.create(memory.get(PERSON_DAVE_AT_CHAT), "works_at", memory.get("org/hooli"))"#)
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
    assert!(result.contains("works_at → org/hooli"), "{result}");
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
