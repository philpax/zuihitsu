use super::*;

/// The `person/dave` handle string a script's `PERSON_DAVE` token resolves to, for asserting the
/// rendered record names the memory.
fn dave_handle() -> String {
    MemoryName::from(Namespace::Person.with_name("dave"))
        .as_str()
        .to_owned()
}

#[tokio::test]
async fn details_renders_the_whole_record() {
    let h = Harness::new();
    // Build a rich record: a relation and a linked topic, a plain entry and a dated one, a tag, and
    // high volatility — every section details() renders.
    let seeded = h
        .run(
            r#"
        links.register({ name = "part_of", inverse = "contains", from_card = "many", to_card = "many" })
        tags.create("hobbies", "Recreational activities and interests")
        local topic = memory.create(TOPIC_CLIMBING, "The climbing crew")
        local dave = memory.create(PERSON_DAVE, "Met at the climbing gym", { visibility = "public" })
        dave:append("Leads 5.11 routes", { visibility = "public" })
        dave:append("Trip to Yosemite", { visibility = "public", occurred_at = calendar.date("2027-05-01") })
        dave:tag("hobbies")
        dave:set_volatility("high")
        dave:link("part_of", topic)
        return "ok"
        "#,
        )
        .await;
    assert!(
        matches!(seeded, BlockOutcome::Committed { .. }),
        "{seeded:?}"
    );

    let BlockOutcome::Committed { result } =
        h.run(r#"return memory.get(PERSON_DAVE):details()"#).await
    else {
        panic!("expected a committed read");
    };

    // The header names the memory.
    assert!(
        result.contains(&dave_handle()),
        "header names the memory: {result}"
    );
    // A count header over the three entries (create's first entry plus two appends).
    assert!(result.contains("3 entries:"), "count header: {result}");
    // Every entry's text, uncapped.
    assert!(result.contains("Met at the climbing gym"), "{result}");
    assert!(result.contains("Leads 5.11 routes"), "{result}");
    assert!(result.contains("Trip to Yosemite"), "{result}");
    // The dated entry carries the same occurrence marker entries() renders.
    assert!(
        result.contains("2027-05-01"),
        "the dated entry should render its occurrence: {result}"
    );
    // The links section, rendered as the link readers show a row.
    assert!(result.contains("links:"), "links section header: {result}");
    assert!(
        result.contains("part_of →"),
        "the link renders as its reader row: {result}"
    );
    // The tags section.
    assert!(result.contains("tags: #hobbies"), "tags section: {result}");
    // The volatility.
    assert!(
        result.contains("volatility: high"),
        "volatility section: {result}"
    );
}

#[tokio::test]
async fn details_marks_a_teller_private_entry_rather_than_omitting_it() {
    // details is the agent's own whole-class read, so a teller-private aside is shown with its
    // visibility and teller markers, not withheld — the agent reads its complete record.
    let h = Harness::new();
    h.run(
        r#"
        local dave = memory.create(PERSON_DAVE)
        dave:append("Prefers to keep the job search quiet", { visibility = "private" })
        return "ok"
        "#,
    )
    .await;

    let BlockOutcome::Committed { result } =
        h.run(r#"return memory.get(PERSON_DAVE):details()"#).await
    else {
        panic!("expected a committed read");
    };
    assert!(
        result.contains("Prefers to keep the job search quiet"),
        "{result}"
    );
    // The entry's private visibility marker rides the render, the same as entries() shows it.
    assert!(
        result.contains("private"),
        "the private marker should render: {result}"
    );
}

#[tokio::test]
async fn details_shows_former_names_after_a_rename() {
    let h = Harness::new();
    h.run(r#"memory.create(PERSON_DAVE, "a durable fact", { visibility = "public" })"#)
        .await;
    h.run(r#"memory.get(PERSON_DAVE):rename(PERSON_SARAH)"#)
        .await;

    let BlockOutcome::Committed { result } =
        h.run(r#"return memory.get(PERSON_SARAH):details()"#).await
    else {
        panic!("expected a committed read");
    };
    // The header carries the former handle so the record reads as the same person, renamed.
    assert!(
        result.contains("formerly"),
        "the header notes the former name: {result}"
    );
    assert!(
        result.contains(&dave_handle()),
        "the former handle is named: {result}"
    );
}

#[tokio::test]
async fn details_omits_empty_link_and_tag_sections_and_reports_default_volatility() {
    // A bare memory with a single entry, no links, and no tags: the count header is singular, the
    // link and tag sections are omitted entirely, and the default volatility still reports.
    let h = Harness::new();
    h.run(r#"memory.create(PERSON_DAVE, "the only fact", { visibility = "public" })"#)
        .await;

    let BlockOutcome::Committed { result } =
        h.run(r#"return memory.get(PERSON_DAVE):details()"#).await
    else {
        panic!("expected a committed read");
    };
    assert!(
        result.contains("1 entry:"),
        "singular count header: {result}"
    );
    assert!(result.contains("the only fact"), "{result}");
    assert!(
        !result.contains("links:"),
        "no link section when there are no links: {result}"
    );
    assert!(
        !result.contains("tags:"),
        "no tag section when there are no tags: {result}"
    );
    assert!(
        result.contains("volatility: medium"),
        "default volatility reports: {result}"
    );
}
