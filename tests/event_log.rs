//! Event-log seam tests, written once against the `Store` trait and run against every backend, so
//! the in-memory and SQLite stores are held to the same total-order and faithful-replay contract.

use zuihitsu::{EventPayload, MemoryId, MemoryName, MemoryStore, Seq, Store, TagName, Timestamp};

fn sample_payloads() -> Vec<EventPayload> {
    let id = MemoryId::generate();
    vec![
        EventPayload::TagCreated {
            name: TagName::new("hobbies"),
            description: "Recreational activities and interests".to_owned(),
        },
        EventPayload::MemoryCreated {
            id,
            name: MemoryName::new("person/dave"),
        },
        EventPayload::MemoryRenamed {
            id,
            old_name: MemoryName::new("person/dave"),
            new_name: MemoryName::new("person/dave-chen"),
        },
    ]
}

/// Appending stamps consecutive sequence numbers, and a full read returns the exact payloads in
/// the exact order they were committed — faithful replay at the log layer.
fn append_is_ordered_and_faithful<S: Store>(store: &mut S) {
    assert_eq!(store.head().unwrap(), Seq::ZERO);

    let payloads = sample_payloads();
    let committed = store
        .append(Timestamp::from_millis(1_000), payloads.clone())
        .unwrap();

    assert_eq!(committed.len(), 3);
    assert_eq!(committed[0].seq, Seq(1));
    assert_eq!(committed[2].seq, Seq(3));
    assert_eq!(store.head().unwrap(), Seq(3));

    let replayed = store.read_from(Seq::ZERO).unwrap();
    let replayed_payloads: Vec<EventPayload> = replayed.iter().map(|e| e.payload.clone()).collect();
    assert_eq!(replayed_payloads, payloads);
    assert!(replayed.windows(2).all(|w| w[0].seq < w[1].seq));
    assert!(
        replayed
            .iter()
            .all(|e| e.recorded_at == Timestamp::from_millis(1_000))
    );
}

/// Two appends continue the sequence, and `read_from` returns only the requested tail.
fn read_from_returns_tail<S: Store>(store: &mut S) {
    store
        .append(
            Timestamp::from_millis(1),
            vec![EventPayload::MemoryDeleted {
                id: MemoryId::generate(),
            }],
        )
        .unwrap();
    store
        .append(
            Timestamp::from_millis(2),
            vec![EventPayload::MemoryDeleted {
                id: MemoryId::generate(),
            }],
        )
        .unwrap();

    let tail = store.read_from(Seq(2)).unwrap();
    assert_eq!(tail.len(), 1);
    assert_eq!(tail[0].seq, Seq(2));
}

/// A subscriber taken before an append receives the committed events.
fn subscriber_sees_appends<S: Store>(store: &mut S) {
    let subscription = store.subscribe();
    store
        .append(
            Timestamp::from_millis(5),
            vec![EventPayload::TagCreated {
                name: TagName::new("colleagues"),
                description: "People worked with".to_owned(),
            }],
        )
        .unwrap();

    let event = subscription.recv().unwrap();
    assert_eq!(event.seq, Seq(1));
    assert_eq!(event.payload.kind(), "TagCreated");
}

#[test]
fn memory_append_is_ordered_and_faithful() {
    append_is_ordered_and_faithful(&mut MemoryStore::new());
}

#[test]
fn memory_read_from_returns_tail() {
    read_from_returns_tail(&mut MemoryStore::new());
}

#[test]
fn memory_subscriber_sees_appends() {
    subscriber_sees_appends(&mut MemoryStore::new());
}

#[cfg(feature = "sqlite")]
mod sqlite {
    use super::*;
    use zuihitsu::SqliteStore;

    #[test]
    fn append_is_ordered_and_faithful() {
        super::append_is_ordered_and_faithful(&mut SqliteStore::open_in_memory().unwrap());
    }

    #[test]
    fn read_from_returns_tail() {
        super::read_from_returns_tail(&mut SqliteStore::open_in_memory().unwrap());
    }

    #[test]
    fn subscriber_sees_appends() {
        super::subscriber_sees_appends(&mut SqliteStore::open_in_memory().unwrap());
    }

    /// The log survives a process boundary: append, drop, reopen, and the events are still there in
    /// order — the property the whole "rebuild from the log" story rests on.
    #[test]
    fn persists_across_reopen() {
        let path =
            std::env::temp_dir().join(format!("zuihitsu-test-{}.sqlite", MemoryId::generate().0));

        {
            let mut store = SqliteStore::open(&path).unwrap();
            store
                .append(Timestamp::from_millis(1_000), sample_payloads())
                .unwrap();
        }
        {
            let store = SqliteStore::open(&path).unwrap();
            assert_eq!(store.head().unwrap(), Seq(3));
            assert_eq!(store.read_from(Seq::ZERO).unwrap().len(), 3);
        }

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }
}
