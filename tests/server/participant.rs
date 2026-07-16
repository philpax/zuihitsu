use zuihitsu::LinkNode;

use super::*;

/// Collect the live entry texts of a memory, for asserting what a projection left on the profile.
fn entry_texts(server: &Server, name: &str) -> Vec<String> {
    server
        .control()
        .entries(name)
        .unwrap()
        .into_iter()
        .map(|entry| entry.text)
        .collect()
}

#[tokio::test]
async fn projecting_identity_appends_supersedes_and_retracts() {
    let (server, _clock) = born_agent();
    let dave = PersonId::new(TEST_PLATFORM, "dave");
    let profile = "person/dave@chat";

    // First contact: both attributes are recorded fresh (nothing to supersede), and their entry ids
    // come back for the connector to hold.
    let ids = server
        .platform()
        .project(
            &LinkNode::Participant(dave.clone()),
            "discord",
            &[
                ParticipantAttribute {
                    text: Some("Discord username: dave1234".to_owned()),
                    supersedes: None,
                },
                ParticipantAttribute {
                    text: Some("Discord display name: Dave".to_owned()),
                    supersedes: None,
                },
            ],
        )
        .unwrap();
    let (username_id, display_id) = (ids[0].unwrap(), ids[1].unwrap());
    let texts = entry_texts(&server, profile);
    assert!(texts.contains(&"Discord username: dave1234".to_owned()));
    assert!(texts.contains(&"Discord display name: Dave".to_owned()));

    // The username changes: the new value supersedes the entry the first projection returned, so only
    // the new one is live.
    let ids = server
        .platform()
        .project(
            &LinkNode::Participant(dave.clone()),
            "discord",
            &[ParticipantAttribute {
                text: Some("Discord username: dave5678".to_owned()),
                supersedes: Some(username_id),
            }],
        )
        .unwrap();
    let new_username_id = ids[0].unwrap();
    let texts = entry_texts(&server, profile);
    assert!(texts.contains(&"Discord username: dave5678".to_owned()));
    assert!(
        !texts.contains(&"Discord username: dave1234".to_owned()),
        "the prior username is superseded, not live: {texts:?}"
    );
    assert!(texts.contains(&"Discord display name: Dave".to_owned()));

    // The display name is cleared: the projection carries no text, so its entry is retracted.
    let ids = server
        .platform()
        .project(
            &LinkNode::Participant(dave.clone()),
            "discord",
            &[ParticipantAttribute {
                text: None,
                supersedes: Some(display_id),
            }],
        )
        .unwrap();
    assert_eq!(ids, vec![None], "a cleared attribute returns no new entry");
    let texts = entry_texts(&server, profile);
    assert!(
        !texts.contains(&"Discord display name: Dave".to_owned()),
        "the cleared display name is retracted: {texts:?}"
    );
    assert_eq!(
        texts,
        vec!["Discord username: dave5678".to_owned()],
        "only the current username remains live"
    );

    // The entries are attributed to the connector, not the agent.
    let projected = server.control().events().unwrap().into_iter().any(|event| {
        matches!(&event.source, EventSource::Connector(id) if id == "discord")
            && matches!(&event.payload, EventPayload::MemoryContentAppended { text, .. }
                if text == "Discord username: dave5678")
    });
    assert!(projected, "the projection is attributed to the connector");

    // A later supersede whose target the agent has since dropped is a no-op — the fresh value still
    // lands rather than the whole projection failing.
    let ids = server
        .platform()
        .project(
            &LinkNode::Participant(dave.clone()),
            "discord",
            &[ParticipantAttribute {
                text: Some("Discord username: dave9999".to_owned()),
                supersedes: Some(new_username_id),
            }],
        )
        .unwrap();
    assert!(ids[0].is_some());
}

#[tokio::test]
async fn projecting_no_attributes_is_a_no_op() {
    let (server, _clock) = born_agent();
    let dave = PersonId::new(TEST_PLATFORM, "dave");
    let ids = server
        .platform()
        .project(&LinkNode::Participant(dave.clone()), "discord", &[])
        .unwrap();
    assert!(ids.is_empty());
    // No stub is minted for an empty projection.
    assert!(
        server
            .control()
            .memory("person/dave@chat")
            .unwrap()
            .is_none()
    );
}
