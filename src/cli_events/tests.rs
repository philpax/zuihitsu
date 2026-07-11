use super::*;
use anstyle::AnsiColor;
use zuihitsu::{
    Event, EventPayload, MemoryId, Seq, Volatility, event::EventSource, ids::MemoryName,
    time::Timestamp, vocabulary::TagName,
};

fn ev(seq: u64, payload: EventPayload) -> Event {
    Event {
        seq: Seq(seq),
        recorded_at: Timestamp::from_millis(0),
        source: EventSource::Agent,
        payload,
    }
}

#[test]
fn name_map_resolves_a_create_and_the_latest_rename() {
    let id = MemoryId::generate();
    let events = vec![
        ev(
            1,
            EventPayload::memory_created(id, MemoryName::new("person/dave")),
        ),
        ev(
            2,
            EventPayload::memory_renamed(
                id,
                MemoryName::new("person/dave"),
                MemoryName::new("person/sarah"),
            ),
        ),
    ];
    let names = name_map(&events);
    assert_eq!(
        names.get(&id.0.to_string()).map(String::as_str),
        Some("person/sarah")
    );
}

#[test]
fn describe_event_glosses_payloads_resolving_ids_to_names() {
    let id = MemoryId::generate();
    let names = name_map(&[ev(
        1,
        EventPayload::memory_created(id, MemoryName::new("person/dave")),
    )]);

    assert_eq!(
        describe_event(
            &EventPayload::memory_created(id, MemoryName::new("person/dave")),
            &names
        ),
        "created person/dave"
    );
    assert_eq!(
        describe_event(
            &EventPayload::memory_volatility_set(id, Volatility::High),
            &names
        ),
        "person/dave: volatility high"
    );
    assert_eq!(
        describe_event(
            &EventPayload::tag_applied_to_memory(id, TagName::new("hobbies")),
            &names
        ),
        "person/dave +#hobbies"
    );
    // A memory the log never names falls back to a short id rather than panicking.
    let other = MemoryId::generate();
    assert!(describe_event(&EventPayload::memory_deleted(other), &names).starts_with("deleted …"));
}

#[test]
fn category_color_groups_by_kind() {
    let id = MemoryId::generate();
    assert_eq!(
        category_color(&EventPayload::memory_created(id, MemoryName::new("self"))),
        AnsiColor::BrightGreen
    );
    assert_eq!(
        category_color(&EventPayload::tag_created(TagName::new("x"), "d")),
        AnsiColor::Yellow
    );
    assert_eq!(
        category_color(&EventPayload::memory_deleted(id)),
        AnsiColor::BrightGreen
    );
}

#[test]
fn json_listing_is_ndjson_that_round_trips() {
    let event = ev(
        7,
        EventPayload::memory_created(MemoryId::generate(), MemoryName::new("person/dave")),
    );
    let line = serde_json::to_string(&event).unwrap();
    assert!(!line.contains('\n'), "one event must serialize to one line");
    let parsed: Event = serde_json::from_str(&line).unwrap();
    assert_eq!(parsed.seq, Seq(7));
    assert_eq!(parsed.payload.kind(), "MemoryCreated");
}
