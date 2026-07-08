use super::*;
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
