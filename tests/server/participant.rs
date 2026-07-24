use zuihitsu::LinkNode;

use crate::{
    EventPayload, EventSource, MemoryId, ParticipantAttribute, PersonId, Server, TEST_PLATFORM,
    born_agent,
};

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
    // The response names the memory the projection landed on — what a connector holds to reference the
    // subject (a `[mem:<id>]` splice) — alongside the per-attribute entry ids.
    assert_eq!(
        ids.memory_id,
        server.control().memory(profile).unwrap().unwrap().id,
        "the response names the memory the projection landed on"
    );
    let (username_id, display_id) = (ids.entries[0].unwrap(), ids.entries[1].unwrap());
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
    let new_username_id = ids.entries[0].unwrap();
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
    assert_eq!(
        ids.entries,
        vec![None],
        "a cleared attribute returns no new entry"
    );
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
        matches!(&event.source, EventSource::PlatformConnector(id) if id == "discord")
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
    assert!(ids.entries[0].is_some());
}

#[tokio::test]
async fn projecting_no_attributes_returns_the_resolved_memory_id() {
    let (server, _clock) = born_agent();
    let dave = PersonId::new(TEST_PLATFORM, "dave");
    let outcome = server
        .platform()
        .project(&LinkNode::Participant(dave.clone()), "discord", &[])
        .unwrap();
    // No attributes means no content entries, but the subject's memory id still comes back — resolved
    // (and minted on first contact), so a connector can learn a subject's memory id to reference it
    // without recording anything.
    assert!(outcome.entries.is_empty(), "no attributes, no entries");
    let stub = server
        .control()
        .memory("person/dave@chat")
        .unwrap()
        .expect("the subject stub is resolved and minted");
    assert_eq!(
        outcome.memory_id, stub.id,
        "the response names the resolved memory"
    );
}

/// How many `MemoryCreated` events named `name` the log holds — a double-minted stub shows as two.
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_first_contact_mints_one_stub() {
    // Racing first-contact projections for the same never-seen participant must resolve to a single
    // stub. The resolve-miss, mint, and materialize have to be atomic under the graph lock: released
    // between the mint's append and the materialize, every racer misses and each appends a
    // `MemoryCreated` under the same qualified name, and folding that pair collides on the
    // `memories.name` UNIQUE index — wedging every later materialize and fragmenting the identity
    // across duplicate stubs. The mention feature multiplied how often `/platform/project` fires for a
    // never-seen user, so this window is hit in production, not just in theory.
    //
    // The racers run on real OS threads (`project` is synchronous), so they genuinely contend on the
    // `parking_lot` graph mutex rather than cooperatively interleaving. A barrier aligns their entry,
    // and the trial count makes a pre-fix miss near-certain — the first double-mint wedges the shared
    // graph, so every subsequent projection then errors on the stuck fold.
    let (server, _clock) = born_agent();
    let server = std::sync::Arc::new(server);

    for n in 0..48 {
        let person = PersonId::new(TEST_PLATFORM, format!("racer{n}"));
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let server = std::sync::Arc::clone(&server);
                let barrier = std::sync::Arc::clone(&barrier);
                let person = person.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    server
                        .platform()
                        .project(&LinkNode::Participant(person), "discord", &[])
                        .map(|outcome| outcome.memory_id)
                })
            })
            .collect();
        let ids: Vec<MemoryId> = handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .unwrap()
                    .expect("a first-contact projection must not fail")
            })
            .collect();

        assert!(
            ids.iter().all(|id| *id == ids[0]),
            "every racer resolves to the same stub for racer{n}",
        );
        assert_eq!(
            creations(&server, &format!("person/racer{n}@chat")),
            1,
            "first contact mints exactly one stub for racer{n}",
        );
    }
}
