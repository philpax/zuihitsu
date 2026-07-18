//! Connector-authored structural links via `Platform::link`: a channel or a member placed in a guild,
//! carrying `LinkSource::Connector`, refusing `same_as`, and retracting on departure.

use zuihitsu::{
    LinkError, LinkNode,
    event::{EventPayload, LinkSource, Visibility},
};

use super::*;

/// The `part_of` links in the log, as `(source, visibility)` — one row per `LinkCreated`.
fn part_of_links(server: &Server) -> Vec<(LinkSource, Visibility)> {
    server
        .control()
        .events()
        .unwrap()
        .into_iter()
        .filter_map(|event| match event.payload {
            EventPayload::LinkCreated {
                relation,
                source,
                visibility,
                ..
            } if relation.as_str() == "part_of" => Some((source, visibility)),
            _ => None,
        })
        .collect()
}

/// How many `LinkRemoved` events name `part_of`.
fn part_of_removals(server: &Server) -> usize {
    server
        .control()
        .events()
        .unwrap()
        .into_iter()
        .filter(|event| {
            matches!(&event.payload, EventPayload::LinkRemoved { relation, .. }
                if relation.as_str() == "part_of")
        })
        .count()
}

#[tokio::test]
async fn a_connector_places_a_channel_and_a_member_in_a_guild() {
    let (server, _clock) = born_agent();
    let guild = ConversationLocator::new(TEST_PLATFORM, "guild/42");
    let channel = ConversationLocator::new(TEST_PLATFORM, "guild/42/channel/7");
    let member = PersonId::new(TEST_PLATFORM, "555");

    server
        .platform()
        .link(
            &LinkNode::Context(channel),
            &LinkNode::Context(guild.clone()),
            "part_of",
            "chat",
            false,
        )
        .unwrap();
    server
        .platform()
        .link(
            &LinkNode::Participant(member),
            &LinkNode::Context(guild),
            "part_of",
            "chat",
            false,
        )
        .unwrap();

    // Both edges landed, each a public structural fact attributed to the connector.
    let links = part_of_links(&server);
    assert_eq!(links.len(), 2, "a channel edge and a member edge");
    for (source, visibility) in &links {
        assert_eq!(*source, LinkSource::Connector("chat".to_owned()));
        assert_eq!(*visibility, Visibility::Public);
    }
}

#[tokio::test]
async fn a_connector_may_not_assert_same_as() {
    // Cross-platform identity is operator-confirmed: a connector asserting `same_as` is refused
    // outright, never buffered as a proposal.
    let (server, _clock) = born_agent();
    let a = PersonId::new(TEST_PLATFORM, "1");
    let b = PersonId::new(TEST_PLATFORM, "2");
    let error = server
        .platform()
        .link(
            &LinkNode::Participant(a),
            &LinkNode::Participant(b),
            "same_as",
            "chat",
            false,
        )
        .unwrap_err();
    assert!(matches!(error, LinkError::SameAsForbidden));
}

#[tokio::test]
async fn an_unregistered_relation_is_refused() {
    let (server, _clock) = born_agent();
    let guild = ConversationLocator::new(TEST_PLATFORM, "guild/42");
    let member = PersonId::new(TEST_PLATFORM, "555");
    let error = server
        .platform()
        .link(
            &LinkNode::Participant(member),
            &LinkNode::Context(guild),
            "frobnicates",
            "chat",
            false,
        )
        .unwrap_err();
    assert!(matches!(error, LinkError::UnknownRelation(_)));
}

#[tokio::test]
async fn a_departure_retracts_the_member_link() {
    let (server, _clock) = born_agent();
    let guild = ConversationLocator::new(TEST_PLATFORM, "guild/42");
    let member = PersonId::new(TEST_PLATFORM, "555");

    server
        .platform()
        .link(
            &LinkNode::Participant(member.clone()),
            &LinkNode::Context(guild.clone()),
            "part_of",
            "chat",
            false,
        )
        .unwrap();
    server
        .platform()
        .link(
            &LinkNode::Participant(member),
            &LinkNode::Context(guild),
            "part_of",
            "chat",
            true,
        )
        .unwrap();

    assert_eq!(part_of_links(&server).len(), 1, "the assert");
    assert_eq!(part_of_removals(&server), 1, "the retract");
}

#[tokio::test]
async fn retracting_between_unknown_nodes_is_a_no_op() {
    // A retract naming a guild or member the log has never seen mints nothing and appends nothing —
    // an edge to a node that does not exist cannot exist.
    let (server, _clock) = born_agent();
    let head_before = server.control().events().unwrap().len();
    let guild = ConversationLocator::new(TEST_PLATFORM, "guild/999");
    let member = PersonId::new(TEST_PLATFORM, "ghost");
    server
        .platform()
        .link(
            &LinkNode::Participant(member),
            &LinkNode::Context(guild),
            "part_of",
            "chat",
            true,
        )
        .unwrap();
    assert_eq!(
        server.control().events().unwrap().len(),
        head_before,
        "no mint, no edge, no event"
    );
}
