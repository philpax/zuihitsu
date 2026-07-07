use super::*;

#[tokio::test]
async fn append_carries_teller_context_and_default_visibility() {
    let mut store = MemoryStore::new();
    let clock = ManualClock::new(common::time::EARLY);
    let mut graph = Graph::open_in_memory().unwrap();

    // A room (with its eagerly-minted context memory), the subject, and the speaker.
    let conversation = resolve_or_mint_conversation(
        &mut store,
        &clock,
        &graph,
        &ConversationLocator::new("discord", "leads"),
    )
    .unwrap();
    let marcus = MemoryId::generate();
    let erin = MemoryId::generate();
    store
        .append(
            clock.now(),
            vec![
                EventPayload::memory_created(marcus, Namespace::Person.with_name("marcus")),
                EventPayload::memory_created(erin, Namespace::Person.with_name("erin")),
            ],
        )
        .unwrap();
    graph.materialize_from(&store).unwrap();
    let context = graph
        .context_for_conversation(conversation)
        .unwrap()
        .unwrap();
    let session = Session::new(conversation, InstanceFeatures::default());

    // The shared engine the block writes through, read back below via the same handle.
    let engine = Engine::new(Box::new(store), graph, Box::new(clock.clone()));
    async fn exec(session: &Session, engine: &Arc<Engine>, teller: MemoryId, script: &str) {
        session
            .execute(
                engine,
                &BlockContext {
                    teller: Teller::Participant(teller),
                    authority: Authority::Platform,
                    turn_id: TurnId::generate(),
                    block_timeout: TEST_BLOCK_TIMEOUT,
                    max_block_attempts: TEST_MAX_BLOCK_ATTEMPTS,
                    present_set: Vec::new(),
                    dry_run: false,
                },
                &common::prepare_script(script),
            )
            .await
            .unwrap();
    }

    // Erin, in the room, relays something about Marcus: attributed to her, told in this context, and
    // defaulted private to its teller because the subject (Marcus) is not the teller.
    exec(
        &session,
        &engine,
        erin,
        r#"memory.get(PERSON_MARCUS):append("is being managed out")"#,
    )
    .await;
    // `by_agent` records the agent's own observation about a person, which has no protective default
    // (the aside mechanism keys on a participant teller) — so it must classify the entry explicitly.
    exec(
        &session,
        &engine,
        erin,
        r#"memory.get(PERSON_MARCUS):append("seems stressed", { by_agent = true, visibility = "public" })"#,
    )
    .await;
    exec(
        &session,
        &engine,
        erin,
        r#"memory.get(PERSON_MARCUS):append("got promoted", { visibility = "public" })"#,
    )
    .await;

    let entries = engine.graph.lock().entries_local(marcus).unwrap();
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].told_by, Teller::Participant(erin));
    assert_eq!(entries[0].told_in, Some(context));
    assert_eq!(entries[0].visibility, Visibility::PrivateToTeller);
    assert_eq!(entries[1].told_by, Teller::Agent);
    assert_eq!(entries[1].visibility, Visibility::Public);
    assert_eq!(entries[2].told_by, Teller::Participant(erin));
    assert_eq!(entries[2].visibility, Visibility::Public); // forced, despite the subject mismatch

    // context.current() resolves to this room's context memory.
    exec(
        &session,
        &engine,
        erin,
        r#"context.current():append("kept in confidence", { by_agent = true })"#,
    )
    .await;
    let context_entries = engine.graph.lock().entries_local(context).unwrap();
    assert_eq!(context_entries.len(), 1);
    assert_eq!(context_entries[0].text, "kept in confidence");
}

#[tokio::test]
async fn told_by_stamps_a_relayed_claims_source() {
    // told_by attributes an entry to a teller other than the current speaker: Erin, speaking, relays
    // something Marcus is the source of about Dave. The fact is stamped with Marcus as its provenance,
    // not Erin who relayed it, so it reads and is governed as Marcus's. Both a name string and a handle
    // resolve to the same teller.
    let h = Harness::new();
    h.run(
        r#"memory.create(PERSON_DAVE); memory.create(PERSON_ERIN); memory.create(PERSON_MARCUS)"#,
    )
    .await;
    let id = |name: &str| {
        h.engine
            .graph
            .lock()
            .memory_by_name(MemoryName::new(name))
            .unwrap()
            .unwrap()
            .id
    };
    let (dave, erin, marcus) = (
        id(MemoryName::from(Namespace::Person.with_name("dave")).as_str()),
        id(MemoryName::from(Namespace::Person.with_name("erin")).as_str()),
        id(MemoryName::from(Namespace::Person.with_name("marcus")).as_str()),
    );

    // Erin is the speaker; the first append names Marcus by name string, the second by handle.
    h.run_as(
        Teller::Participant(erin),
        vec![erin],
        r#"
        memory.get(PERSON_DAVE):append("is moving to the Atlas team", { visibility = "public", told_by = PERSON_MARCUS })
        memory.get(PERSON_DAVE):append("prefers async standups", { visibility = "public", told_by = memory.get(PERSON_MARCUS) })
        "#,
    )
    .await;

    let entries = h.engine.graph.lock().entries_local(dave).unwrap();
    assert_eq!(entries.len(), 2);
    // Both are stamped with Marcus (the source), not Erin (the speaker) — via name and via handle.
    assert_eq!(entries[0].told_by, Teller::Participant(marcus));
    assert_eq!(entries[1].told_by, Teller::Participant(marcus));
    assert_ne!(entries[0].told_by, Teller::Participant(erin));
}

#[tokio::test]
async fn told_by_an_unknown_name_is_a_teachable_error() {
    // told_by names a teller that must exist: an unknown name is a teachable error naming the option,
    // not a silent attribution to a nonexistent memory.
    let h = Harness::new();
    h.run(r#"memory.create(PERSON_DAVE)"#).await;
    let outcome = h
        .run(
            r#"memory.get(PERSON_DAVE):append("relayed", { visibility = "public", told_by = PERSON_NOBODY })"#,
        )
        .await;
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = outcome else {
        panic!("expected a teachable error, got {outcome:?}");
    };
    assert!(
        message.contains("told_by") && message.contains("person/nobody"),
        "the error should name told_by and the unknown teller: {message}"
    );
}

#[tokio::test]
async fn a_write_in_a_confidential_room_defaults_private() {
    let mut store = MemoryStore::new();
    let clock = ManualClock::new(common::time::EARLY);
    let mut graph = Graph::open_in_memory().unwrap();

    let conversation = resolve_or_mint_conversation(
        &mut store,
        &clock,
        &graph,
        &ConversationLocator::new("discord", "leads"),
    )
    .unwrap();
    graph.materialize_from(&store).unwrap();
    let context = graph
        .context_for_conversation(conversation)
        .unwrap()
        .unwrap();
    // Mark the room #confidential.
    store
        .append(
            clock.now(),
            vec![
                EventPayload::tag_created(
                    TagName::new("confidential"),
                    "a confidential room".to_owned(),
                ),
                EventPayload::tag_applied_to_memory(context, TagName::new("confidential")),
            ],
        )
        .unwrap();
    graph.materialize_from(&store).unwrap();

    // The agent records a topic in the confidential room. A topic write would normally default
    // public, and the agent teller is always present — but the confidential room forces it private,
    // so it cannot silently surface to whoever is around.
    let session = Session::new(conversation, InstanceFeatures::default());
    let engine = Engine::new(Box::new(store), graph, Box::new(clock.clone()));
    session
        .execute(
            &engine,
            &BlockContext {
                teller: Teller::Agent,
                authority: Authority::Platform,
                turn_id: TurnId::generate(),
                block_timeout: TEST_BLOCK_TIMEOUT,
                max_block_attempts: TEST_MAX_BLOCK_ATTEMPTS,
                present_set: Vec::new(),
                dry_run: false,
            },
            &common::prepare_script(
                r#"memory.create(TOPIC_SENSITIVE, "something said in confidence")"#,
            ),
        )
        .await
        .unwrap();

    let topic = engine
        .graph
        .lock()
        .memory_by_name(Namespace::Topic.with_name("sensitive"))
        .unwrap()
        .unwrap();
    let entries = engine.graph.lock().entries_local(topic.id).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].visibility, Visibility::PrivateToTeller);
}

/// A fact on a memory the agent marked `high` volatility reads as `[stale — no newer entry]` once it
/// ages past the staleness horizon, so the agent hedges rather than asserting it as current — the
/// marker says the fact aged out *and nothing replaced it*, so the agent reconfirms rather than
/// hunting for a fresher version. A default-volatility memory's fact never goes stale. Staleness is
/// age-based and independent of who is present.
#[tokio::test]
async fn a_high_volatility_fact_reads_stale_after_aging() {
    let h = Harness::new();
    h.run(
        r#"
        local d = memory.create(PERSON_DAVE)
        -- Classify volatility inline on the append (the ergonomic path).
        d:append("leads the Atlas project", { visibility = "public", volatility = "high" })
        local p = memory.create("project/atlas")
        p:append("the Atlas project ships in Q3", { visibility = "public" })
        "#,
    )
    .await;
    // Age past the 30-day staleness horizon.
    h.clock.advance_millis(40 * 86_400_000);

    let read = r#"
        local e = memory.get("MEM"):entries()[1]
        return tostring(e.stale) .. "|" .. tostring(e)
    "#;
    let BlockOutcome::Committed { result } = h
        .run(&read.replace(
            "MEM",
            MemoryName::from(Namespace::Person.with_name("dave")).as_str(),
        ))
        .await
    else {
        panic!("expected commit");
    };
    assert!(
        result.starts_with("true|") && result.contains("stale — no newer entry"),
        "the aged high-volatility fact should read `stale — no newer entry`: {result}"
    );
    let BlockOutcome::Committed { result } = h.run(&read.replace("MEM", "project/atlas")).await
    else {
        panic!("expected commit");
    };
    assert!(
        result.starts_with("false|"),
        "a default-volatility fact never goes stale: {result}"
    );
}

/// A superseded aged high-volatility entry — surfaced only by `mem:history`, never a live read —
/// does *not* carry the stale marker: its newer version sits right beside it in the same list, so
/// marking it "no newer entry" would lie. The live tail that aged out with nothing replacing it still
/// reads stale. This is the render distinction the marker's wording promises: `stale — no newer entry`
/// only ever rides an unreplaced fact.
#[tokio::test]
async fn a_superseded_aged_entry_is_not_marked_stale_in_history() {
    let h = Harness::new();
    h.run(
        r#"
        local d = memory.create(PERSON_DAVE)
        d:append("leads the Atlas project", { visibility = "public", volatility = "high" })
        "#,
    )
    .await;
    // Age past the 30-day horizon so the first entry is stale, then supersede it with a newer fact
    // that is itself fresh.
    h.clock.advance_millis(40 * 86_400_000);
    let dave = MemoryName::from(Namespace::Person.with_name("dave"))
        .as_str()
        .to_owned();
    h.run(
        &r#"
        local d = memory.get("MEM")
        local old = d:entries()[1]
        d:revise(old, "now leads the Borealis project", { visibility = "public", volatility = "high" })
        "#
        .replace("MEM", &dave),
    )
    .await;

    let read = r#"
        local d = memory.get("MEM")
        local live = {}
        for _, e in ipairs(d:entries()) do
            live[#live + 1] = tostring(e)
        end
        local past = {}
        for _, e in ipairs(d:history()) do
            past[#past + 1] = tostring(e.stale) .. ":" .. tostring(e)
        end
        return "LIVE=" .. table.concat(live, "|") .. "~~HIST=" .. table.concat(past, "|")
    "#;
    let BlockOutcome::Committed { result } = h.run(&read.replace("MEM", &dave)).await else {
        panic!("expected commit");
    };
    // The live read shows only the fresh successor, unmarked.
    assert!(
        result.contains("LIVE=") && result.contains("now leads the Borealis project"),
        "the live read should surface the fresh successor: {result}"
    );
    assert!(
        !result.split("~~HIST=").next().unwrap().contains("stale"),
        "the live read has no aged-out entry, so nothing is marked stale: {result}"
    );
    // History keeps the superseded entry, but it is not marked stale — its successor is right there.
    let history = result.split("~~HIST=").nth(1).unwrap();
    assert!(
        history.contains("false:") && history.contains("leads the Atlas project"),
        "history keeps the superseded entry, unmarked (it has a successor): {result}"
    );
    assert!(
        !history.contains("stale"),
        "a superseded aged entry must not read stale — there IS a newer entry: {result}"
    );
}

/// An `Attributed` fact — an ordinary thing a colleague relayed — survives the teller's absence: a
/// direct read by a present outsider sees it in full (unlike a confidence, which is withheld), so the
/// agent can still answer "what's Dave's role?" months later in another room. It reads as attributed,
/// carrying its provenance, never as a confidence.
#[tokio::test]
async fn an_attributed_fact_survives_the_teller_absence() {
    let h = Harness::new();
    h.run(r#"memory.create(PERSON_DAVE); memory.create(PERSON_ERIN)"#)
        .await;
    let id = |name: &str| {
        h.engine
            .graph
            .lock()
            .memory_by_name(MemoryName::new(name))
            .unwrap()
            .unwrap()
            .id
    };
    let (dave, erin) = (
        id(MemoryName::from(Namespace::Person.with_name("dave")).as_str()),
        id(MemoryName::from(Namespace::Person.with_name("erin")).as_str()),
    );

    // Erin, present, relays an ordinary fact about Dave (attributed) and a genuine confidence (private).
    h.run_as(
        Teller::Participant(erin),
        vec![erin],
        r#"
        memory.get(PERSON_DAVE):append("Engineering lead at Hooli", { visibility = "attributed" })
        memory.get(PERSON_DAVE):append("quietly interviewing elsewhere", { visibility = "private" })
        "#,
    )
    .await;

    // A different person (dave himself) present, the teller (erin) absent: the attributed fact stands
    // in full and reads as attributed; the confidence is withheld.
    let read = r#"
        local lines = {}
        for _, e in ipairs(memory.get(PERSON_DAVE):entries()) do
            lines[#lines + 1] = e.visibility .. "/" .. tostring(e.withheld) .. ":" .. e.text
        end
        return table.concat(lines, "|")
    "#;
    let BlockOutcome::Committed { result } = h.run_as(Teller::Agent, vec![dave], read).await else {
        panic!("expected commit");
    };
    assert!(
        result.contains("attributed/false:Engineering lead at Hooli"),
        "the attributed fact should survive the teller's absence, in full: {result}"
    );
    assert!(
        result.contains("private/true:(withheld") && !result.contains("interviewing elsewhere"),
        "the confidence should still be withheld from an outsider: {result}"
    );
}

/// A direct read withholds a confidence from a present audience that is not cleared to see it — the
/// same predicate search applies, now on `mem:entries`/`mem:history`. This closes the name-conflation
/// leak: reading `person/dave` while someone *other* than Dave is present must not hand over Dave's
/// confidence. A public fact is never withheld; with no one present the agent sees everything.
#[tokio::test]
async fn a_direct_read_withholds_a_confidence_from_a_present_outsider() {
    let h = Harness::new();
    h.run(r#"memory.create(PERSON_DAVE); memory.create(PERSON_ERIN)"#)
        .await;
    let id = |name: &str| {
        h.engine
            .graph
            .lock()
            .memory_by_name(MemoryName::new(name))
            .unwrap()
            .unwrap()
            .id
    };
    let (dave, erin) = (
        id(MemoryName::from(Namespace::Person.with_name("dave")).as_str()),
        id(MemoryName::from(Namespace::Person.with_name("erin")).as_str()),
    );

    // Dave, present, confides something private and states a public fact.
    h.run_as(
        Teller::Participant(dave),
        vec![dave],
        r#"
        memory.get(PERSON_DAVE):append("interviewing at a competitor", { visibility = "private" })
        memory.get(PERSON_DAVE):append("runs the Berlin marathon", { visibility = "public" })
        "#,
    )
    .await;

    // A read script that reports each entry as "<withheld>:<text>", oldest first.
    let read = r#"
        local lines = {}
        for _, e in ipairs(memory.get(PERSON_DAVE):entries()) do
            lines[#lines + 1] = tostring(e.withheld) .. ":" .. e.text
        end
        return table.concat(lines, "|")
    "#;

    // (a) Erin present, Dave absent: the confidence is withheld to a stub; the public fact stands.
    let BlockOutcome::Committed { result } = h.run_as(Teller::Agent, vec![erin], read).await else {
        panic!("expected commit");
    };
    assert!(
        result.contains("true:(withheld"),
        "the confidence should be withheld from Erin: {result}"
    );
    assert!(
        !result.contains("interviewing at a competitor"),
        "the confidence text must not reach a read while only Erin is present: {result}"
    );
    assert!(
        result.contains("false:runs the Berlin marathon"),
        "the public fact should stand: {result}"
    );

    // (b) Dave himself present: his own confidence surfaces in full.
    let BlockOutcome::Committed { result } = h.run_as(Teller::Agent, vec![dave], read).await else {
        panic!("expected commit");
    };
    assert!(
        result.contains("false:interviewing at a competitor"),
        "Dave present should see his own confidence: {result}"
    );

    // (c) No one present (a solo flush or maintenance read): the agent sees its whole memory.
    let BlockOutcome::Committed { result } = h.run_as(Teller::Agent, Vec::new(), read).await else {
        panic!("expected commit");
    };
    assert!(
        result.contains("false:interviewing at a competitor"),
        "a solo read is unredacted: {result}"
    );

    // (d) History redacts on the same rule, even though it shows superseded entries — Erin present.
    let history = r#"
        local lines = {}
        for _, e in ipairs(memory.get(PERSON_DAVE):history()) do
            lines[#lines + 1] = tostring(e.withheld) .. ":" .. e.text
        end
        return table.concat(lines, "|")
    "#;
    let BlockOutcome::Committed { result } = h.run_as(Teller::Agent, vec![erin], history).await
    else {
        panic!("expected commit");
    };
    assert!(
        result.contains("true:(withheld") && !result.contains("interviewing at a competitor"),
        "history withholds the confidence from Erin too: {result}"
    );
}
