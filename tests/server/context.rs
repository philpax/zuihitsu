use super::*;

/// How many `MemoryCreated` events named `name` the log holds — a duplicate context memory would show
/// as two.
fn creations(server: &Server, name: &str) -> usize {
    server
        .control()
        .events()
        .unwrap()
        .into_iter()
        .filter(|event| {
            matches!(&event.payload, EventPayload::MemoryCreated { name: created, .. }
                if created.as_str() == name)
        })
        .count()
}

#[tokio::test]
async fn a_standalone_context_needs_no_conversation() {
    // A guild has channels but no messages of its own; its context memory is minted by scope alone,
    // with no phantom conversation behind it.
    let (server, _clock) = born_agent();
    let guild = ConversationLocator::new(TEST_PLATFORM, "guild/42");
    server
        .platform()
        .write_context(
            &guild,
            "discord",
            &[ContextEntry {
                text: "Guild: Acme".to_owned(),
            }],
        )
        .unwrap();

    assert!(
        server
            .control()
            .memory("context/chat:guild/42")
            .unwrap()
            .is_some()
    );
    assert!(
        server.control().sessions(&guild).unwrap().is_empty(),
        "a standalone context opens no conversation"
    );
    assert_eq!(creations(&server, "context/chat:guild/42"), 1);
}

#[tokio::test]
async fn a_context_written_before_the_first_message_is_reused_by_the_conversation() {
    // A connector establishes a channel's context before anyone speaks; the first message opens the
    // conversation over that same context memory rather than minting a duplicate.
    let (server, _clock) = born_agent();
    let leads = ConversationLocator::new(TEST_PLATFORM, "leads");
    server
        .platform()
        .write_context(
            &leads,
            "discord",
            &[ContextEntry {
                text: "Channel: #leads".to_owned(),
            }],
        )
        .unwrap();
    let before = server
        .control()
        .memory("context/chat:leads")
        .unwrap()
        .unwrap()
        .id;

    // Opening the conversation (the same resolve path a first message takes) reuses the context
    // memory by name rather than minting a duplicate.
    server.platform().ensure_conversation(&leads).unwrap();

    let after = server
        .control()
        .memory("context/chat:leads")
        .unwrap()
        .unwrap()
        .id;
    assert_eq!(
        before, after,
        "the conversation reuses the pre-written context memory"
    );
    assert_eq!(
        creations(&server, "context/chat:leads"),
        1,
        "no duplicate context memory across the two paths"
    );
}
