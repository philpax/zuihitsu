use super::*;

#[tokio::test]
async fn list_matches_a_stem_alphabetically() {
    let h = Harness::new();
    h.run(
        r#"
        memory.create(PERSON_DAVE)
        memory.create("person/david")
        memory.create(PERSON_ERIN)
        memory.create(TOPIC_CLIMBING)
        return "ok"
        "#,
    )
    .await;

    let BlockOutcome::Committed { result } = h.run(r#"return memory.list("person/")"#).await else {
        panic!("expected a committed read");
    };
    // Only the person/ stem matches, and in alphabetical order.
    assert!(result.contains("person/dave"), "{result}");
    assert!(result.contains("person/david"), "{result}");
    assert!(result.contains("person/erin"), "{result}");
    assert!(
        !result.contains("topic/climbing"),
        "the topic stem must not match: {result}"
    );
    let dave = result.find("person/dave").unwrap();
    let david = result.find("person/david").unwrap();
    let erin = result.find("person/erin").unwrap();
    assert!(
        dave < david && david < erin,
        "results are alphabetical: {result}"
    );
}

#[tokio::test]
async fn list_returns_a_plain_iterable_sequence_of_handles() {
    // The value is a plain sequence of handles the agent iterates — each handle's name reads lazily,
    // and the truncation note never leaks into iteration.
    let h = Harness::new();
    h.run(
        r#"
        memory.create(PERSON_DAVE)
        memory.create("person/david")
        return "ok"
        "#,
    )
    .await;

    let BlockOutcome::Committed { result } = h
        .run(
            r#"
        local names = {}
        for _, m in ipairs(memory.list("person/")) do
            table.insert(names, m.name)
        end
        return table.concat(names, ",")
        "#,
        )
        .await
    else {
        panic!("expected a committed read");
    };
    assert_eq!(result, "person/dave,person/david");
}

#[tokio::test]
async fn list_caps_the_results_and_notes_the_remainder() {
    let h = Harness::new();
    // Fifty-five zero-padded handles under one stem, so the cap of fifty elides five.
    let mut seed = String::new();
    for i in 1..=55 {
        seed.push_str(&format!("memory.create(\"person/p{i:02}\")\n"));
    }
    seed.push_str("return \"ok\"\n");
    h.run(&seed).await;

    // The returned value is the capped sequence — exactly fifty handles.
    let BlockOutcome::Committed { result: count } =
        h.run(r#"return #memory.list("person/p")"#).await
    else {
        panic!("expected a committed read");
    };
    assert_eq!(count, "50", "the sequence is capped at fifty");

    // The rendered form carries the remainder note.
    let BlockOutcome::Committed { result } = h.run(r#"return memory.list("person/p")"#).await
    else {
        panic!("expected a committed read");
    };
    assert!(
        result.contains("(+5 more — narrow the prefix)"),
        "the render notes the elided handles: {result}"
    );
}

#[tokio::test]
async fn list_matches_a_percent_literally_not_as_a_wildcard() {
    // The prefix's LIKE metacharacters are escaped, so a "%" in the stem matches a literal percent
    // sign rather than wildcarding the rest of the name.
    let h = Harness::new();
    h.run(
        r#"
        memory.create(PERSON_DAVE)
        memory.create("person/%special")
        return "ok"
        "#,
    )
    .await;

    let BlockOutcome::Committed { result } = h.run(r#"return memory.list("person/%")"#).await
    else {
        panic!("expected a committed read");
    };
    assert!(
        result.contains("person/%special"),
        "the literal-% name matches: {result}"
    );
    assert!(
        !result.contains("person/dave"),
        "a % must not wildcard — person/dave must not match \"person/%\": {result}"
    );
}

#[tokio::test]
async fn list_with_a_blank_prefix_is_a_teachable_error() {
    let h = Harness::new();
    for blank in [r#"return memory.list("")"#, r#"return memory.list("   ")"#] {
        let outcome = h.run(blank).await;
        let BlockOutcome::Terminated(TerminalCause::Error(message)) = outcome else {
            panic!("expected a teachable error, got {outcome:?}");
        };
        assert!(
            message.contains("pass a name prefix"),
            "the error names the shape: {message}"
        );
        assert!(
            message.contains("memory.search"),
            "the error points at search for recall-by-meaning: {message}"
        );
    }
}
