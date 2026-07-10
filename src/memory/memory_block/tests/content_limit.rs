//! The entry-length limit enforcement.

use super::{AppendOptions, MemoryError, block_with_limit};
use crate::{
    clock::ManualClock, event::EventPayload, graph::Graph, ids::Namespace, time::Timestamp,
};

#[test]
fn content_too_long_display_message_names_length_and_limit() {
    // The teachable message names the entry's length and the limit, and guides the agent to
    // summarize — so the agent reads the cause and corrects rather than guessing.
    let error = MemoryError::ContentTooLong {
        length: 2048,
        limit: 1000,
    };
    let message = error.to_string();
    assert!(
        message.contains("2048"),
        "the message should name the entry's length: {message}"
    );
    assert!(
        message.contains("1000"),
        "the message should name the limit: {message}"
    );
    assert!(
        message.contains("summarize"),
        "the message should guide the agent to summarize: {message}"
    );
}

#[test]
fn append_rejects_oversized_content() {
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let mut block = block_with_limit(graph, clock, 10);
    let topic = block.create(Namespace::Topic.with_name("a"), None).unwrap();
    let oversized = "x".repeat(11);
    let error = block
        .append(topic, &oversized, AppendOptions::default())
        .unwrap_err();
    assert!(
        matches!(error, MemoryError::ContentTooLong { length, limit } if length == 11 && limit == 10),
        "expected ContentTooLong with length 11 and limit 10, got {error:?}"
    );
    // Nothing was buffered — the rejection happened before the push.
    let effects = block.into_effects();
    assert!(
        !effects
            .events
            .iter()
            .any(|event| matches!(event, EventPayload::MemoryContentAppended { .. })),
        "no content entry should have been buffered"
    );
}

#[test]
fn append_accepts_at_limit() {
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let mut block = block_with_limit(graph, clock, 10);
    let topic = block.create(Namespace::Topic.with_name("a"), None).unwrap();
    // Exactly at the limit — not exceeding it — so the append succeeds.
    let at_limit = "x".repeat(10);
    let result = block.append(topic, &at_limit, AppendOptions::default());
    assert!(result.is_ok(), "an entry at the limit should be accepted");
}

#[test]
fn create_rejects_oversized_first_entry() {
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let mut block = block_with_limit(graph, clock, 10);
    let oversized = "x".repeat(11);
    let error = block
        .create(Namespace::Topic.with_name("a"), Some(&oversized))
        .unwrap_err();
    assert!(
        matches!(error, MemoryError::ContentTooLong { length, limit } if length == 11 && limit == 10),
        "expected ContentTooLong, got {error:?}"
    );
    // The create ran in a transaction, so the rolled-back create leaves the buffer empty of
    // MemoryCreated events.
    let effects = block.into_effects();
    assert!(
        !effects
            .events
            .iter()
            .any(|event| matches!(event, EventPayload::MemoryCreated { .. })),
        "no MemoryCreated should have been buffered after a rolled-back create"
    );
}

#[test]
fn revise_rejects_oversized_replacement() {
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let mut block = block_with_limit(graph, clock, 10);
    let topic = block.create(Namespace::Topic.with_name("a"), None).unwrap();
    let original = block
        .append(topic, "original", AppendOptions::default())
        .unwrap();
    let oversized = "x".repeat(11);
    let error = block
        .revise(topic, original, &oversized, AppendOptions::default())
        .unwrap_err();
    assert!(
        matches!(error, MemoryError::ContentTooLong { length, limit } if length == 11 && limit == 10),
        "expected ContentTooLong, got {error:?}"
    );
    // The revise ran in a transaction, so the oversized append was rolled back — only the original
    // entry remains on the memory.
    let effects = block.into_effects();
    let appended: Vec<&EventPayload> = effects
        .events
        .iter()
        .filter(
            |event| matches!(event, EventPayload::MemoryContentAppended { id, .. } if *id == topic),
        )
        .collect();
    assert_eq!(
        appended.len(),
        1,
        "the failed revise's append should have been rolled back, leaving only the original"
    );
}
