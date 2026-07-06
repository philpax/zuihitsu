use super::*;

#[tokio::test]
async fn interpolation_renders_a_handle_list_element() {
    // Composing text from a reader's list is interpolation's job under Luau: a backtick string
    // stringifies an entry handle through its __tostring, so `{es[1]}` renders the entry's own text —
    // the natural, crash-free way to fold a recalled fact into a reply.
    let h = Harness::new();
    let outcome = h
        .run(
            r#"
        local dave = memory.create(PERSON_DAVE)
        dave:append("Met at the climbing gym", { visibility = "public" })
        dave:append("Got a new job at Hooli", { visibility = "public" })
        local es = dave:entries()
        return `first: {es[1]} || second: {es[2]}`
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert!(result.contains("Met at the climbing gym"), "got: {result}");
    assert!(result.contains("Got a new job at Hooli"), "got: {result}");
    assert!(
        result.contains("first:") && result.contains("|| second:"),
        "the surrounding literal text should frame the interpolated entries, got: {result}"
    );
}

#[tokio::test]
async fn table_concat_on_a_handle_list_is_a_teachable_error() {
    // Stock Luau table.concat joins only strings and numbers, so a reader's handle list fails it with
    // the opaque "invalid value (table) at index 1 in table for 'concat'". The thin error shell keeps
    // stock semantics but rewrites that error, pointing the agent at interpolation instead.
    let h = Harness::new();
    let outcome = h
        .run(
            r#"
        local dave = memory.create(PERSON_DAVE)
        dave:append("Met at the climbing gym", { visibility = "public" })
        return table.concat(dave:entries(), " || ")
        "#,
        )
        .await;
    match outcome {
        BlockOutcome::Terminated(TerminalCause::Error(message)) => {
            assert!(
                message.contains("interpolate") && message.contains("backtick"),
                "the concat-of-handles error should redirect to interpolation, got: {message}"
            );
        }
        other => panic!("expected a teachable error, got {other:?}"),
    }
}

#[tokio::test]
async fn table_concat_on_a_reader_method_is_a_teachable_error() {
    // The field-vs-method slip: the agent passes hub.links (the method itself, a function) to
    // table.concat instead of hub:links() (its result). The error shell catches the non-table first
    // argument and redirects to the colon call rather than surfacing an opaque Lua error.
    let h = Harness::new();
    h.run(r#"memory.create(TOPIC_A)"#).await;
    let outcome = h
        .run(r#"return table.concat(memory.get(TOPIC_A).links, ", ")"#)
        .await;
    match outcome {
        BlockOutcome::Terminated(TerminalCause::Error(message)) => {
            assert!(
                message.contains("call it with a colon"),
                "message was: {message}"
            );
        }
        other => panic!("expected a teachable error, got {other:?}"),
    }
}

#[tokio::test]
async fn table_concat_preserves_stock_behavior_on_primitives() {
    // The error shell must not disturb ordinary joins: it delegates to stock concat, so strings and
    // numbers join as before, the default separator is empty, and the optional i/j range still selects
    // a sub-span.
    let h = Harness::new();
    let outcome = h
        .run(
            r#"
        return table.concat({ "a", "b", "c" }, "-")
            .. " | " .. table.concat({ 1, 2, 3 })
            .. " | " .. table.concat({ "a", "b", "c", "d" }, ",", 2, 3)
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert_eq!(result, "a-b-c | 123 | b,c");
}

#[tokio::test]
async fn memory_and_link_handles_concatenate_as_their_rendered_text() {
    // `"Topic: " .. topic` and `"- " .. link` are the joins the agent actually writes when composing
    // a reply from a read (the Luau validation's residual crashes) — a memory handle and a link
    // result now concatenate as the same text printing shows, from either side, instead of erroring
    // as bare tables.
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

    let BlockOutcome::Committed { result } = h
        .run(
            r#"
        local topic = memory.get(TOPIC_MIGRATION)
        local line = "Topic: " .. topic
        for _, link in ipairs(topic:links()) do
            line = line .. " // " .. ("- " .. link)
        end
        return line
        "#,
        )
        .await
    else {
        panic!("expected a committed read");
    };
    assert!(
        result.starts_with("Topic: ")
            && result.contains(MemoryName::from(Namespace::Topic.with_name("migration")).as_str())
            && result.contains("// - part_of ← "),
        "handles should concatenate as their rendered text, got: {result}"
    );
}

#[tokio::test]
async fn creating_a_duplicate_name_is_a_teachable_error() {
    let h = Harness::new();
    h.run(r#"memory.create(TOPIC_PLAN, "first")"#).await;
    // Re-creating the same name is a teachable block error, not a fatal unique-constraint failure
    // that would poison the log.
    let outcome = h.run(r#"memory.create(TOPIC_PLAN, "second")"#).await;
    match outcome {
        BlockOutcome::Terminated(TerminalCause::Error(message)) => {
            assert!(message.contains("already exists"), "message was: {message}");
            // The wording points at the fetch-or-make idiom for when existence is uncertain.
            assert!(
                message.contains("memory.get_or_create"),
                "message was: {message}"
            );
        }
        other => panic!("expected a teachable error, got {other:?}"),
    }
    // The original memory is intact; the rejected create committed nothing.
    let plan = h
        .engine
        .graph
        .lock()
        .memory_by_name(Namespace::Topic.with_name("plan"))
        .unwrap()
        .unwrap();
    assert_eq!(
        h.engine.graph.lock().entries_local(plan.id).unwrap().len(),
        1
    );
}

#[tokio::test]
async fn get_or_create_returns_the_existing_memory_without_clobbering_it() {
    // `memory.get_or_create` on a name that already exists returns that memory as it stands: the
    // content argument is ignored, so an existing memory's entries are never overwritten or appended
    // to by a fetch-that-happened-to-pass-content.
    let h = Harness::new();
    h.run(r#"memory.create(TOPIC_CLIMBING, "Met at the climbing gym")"#)
        .await;
    let outcome = h
        .run(
            r#"local made = memory.get_or_create(TOPIC_CLIMBING, "this content must be ignored")
               return made:entries()"#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    // The original entry is intact and the ignored content never landed.
    assert!(result.contains("Met at the climbing gym"), "{result}");
    assert!(!result.contains("this content must be ignored"), "{result}");

    // Exactly one entry on the memory — the fetch appended nothing.
    let climbing = h
        .engine
        .graph
        .lock()
        .memory_by_name(Namespace::Topic.with_name("climbing"))
        .unwrap()
        .unwrap();
    assert_eq!(
        h.engine
            .graph
            .lock()
            .entries_local(climbing.id)
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn get_or_create_creates_the_memory_when_it_is_absent() {
    // With no such memory yet, `memory.get_or_create` creates it with the given first entry — the
    // create half of the fetch-or-make idiom.
    let h = Harness::new();
    let outcome = h
        .run(
            r#"local made = memory.get_or_create(TOPIC_SOURDOUGH, "A naturally leavened bread")
               return made:entries()"#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert!(result.contains("naturally leavened"), "{result}");

    // The memory was committed and carries its first entry.
    let sourdough = h
        .engine
        .graph
        .lock()
        .memory_by_name(Namespace::Topic.with_name("sourdough"))
        .unwrap()
        .unwrap();
    assert_eq!(
        h.engine
            .graph
            .lock()
            .entries_local(sourdough.id)
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn assigning_a_field_on_an_entry_handle_is_a_teachable_error() {
    // The read-only guard covers entry handles too — assigning to an entry's field does nothing, so it
    // raises rather than silently dropping the write.
    let h = Harness::new();
    let outcome = h
        .run(
            r#"local topic = memory.create(TOPIC_CLIMBING, "a fact")
               local es = topic:entries()
               es[1].occurred_at = calendar.date("2027-03-15")
               return "unreached""#,
        )
        .await;
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = outcome else {
        panic!("expected a teachable error, got {outcome:?}");
    };
    assert!(
        message.contains("occurred_at is not assignable"),
        "{message}"
    );
}

#[tokio::test]
async fn calling_a_method_with_a_dot_suggests_the_colon() {
    // `dave.append(...)` binds the string to `self`; rather than failing with mlua's opaque "error
    // converting Lua string to table", it raises a teachable error naming the method and the colon
    // call.
    let h = Harness::new();
    let outcome = h
        .run(
            r#"local dave = memory.create(PERSON_DAVE)
               dave.append("a fact", { visibility = "public" })
               return "unreached""#,
        )
        .await;
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = outcome else {
        panic!("expected a teachable error, got {outcome:?}");
    };
    assert!(message.contains("is a method"), "{message}");
    assert!(message.contains("colon"), "{message}");
}

#[tokio::test]
async fn entries_render_as_their_text_and_concatenate() {
    let h = Harness::new();
    // An entry handle reads as its text: returned in a list (rendered for the model) and via `..`.
    let outcome = h
        .run(
            r#"
        local dave = memory.create(PERSON_DAVE)
        dave:append("climbs on Tuesdays", { visibility = "public" })
        local entries = dave:entries()
        return "first: " .. entries[1]
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert!(result.starts_with("first: climbs on Tuesdays"));
}

#[tokio::test]
async fn an_undecorated_table_renders_as_its_structure_not_an_opaque_token() {
    let h = Harness::new();
    // A plain map table the agent builds and returns has no `__tostring`, so before it rendered as
    // the information-free `<table>`. It now pretty-prints through the vendored `inspect`, so the
    // model reads back the fields it returned.
    let outcome = h
        .run(
            r#"
        return { name = "person/dave", role = "climber", visits = 3 }
        "#,
        )
        .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert_ne!(
        result, "<table>",
        "an undecorated table must not render opaquely"
    );
    assert!(
        result.contains("name"),
        "structure should be visible: {result}"
    );
    assert!(
        result.contains(MemoryName::from(Namespace::Person.with_name("dave")).as_str()),
        "values should be visible: {result}"
    );
    assert!(
        result.contains("visits"),
        "every key should be visible: {result}"
    );
}

#[tokio::test]
async fn a_returned_map_renders_nested_handles_as_their_text() {
    // Returning a map of results — `return { list = memory.list(...) }` — goes through the
    // structural inspector, which must render a nested handle as its own text (its name and
    // description read lazily, so the raw structure shows neither) and omit metatable noise: an
    // id-and-metatable blob is illegible at exactly the moment the agent is comparing handles.
    let h = Harness::new();
    h.run(r#"memory.create(PERSON_DAVE)"#).await;
    let BlockOutcome::Committed { result } =
        h.run(r#"return { people = memory.list("person/") }"#).await
    else {
        panic!("expected a committed read");
    };
    assert!(
        result.contains("person/dave"),
        "the nested handle should render its name, got: {result}"
    );
    assert!(
        !result.contains("<metatable>") && !result.contains("__tostring"),
        "metatable noise should be omitted, got: {result}"
    );
}
