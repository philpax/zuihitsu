//! Genesis and boot tests: a fresh log rolls out a complete agent, an interrupted one resumes by
//! emitting only what's missing, and a complete one is left alone — all keyed on the presence of
//! GenesisCompleted, never log emptiness (spec §Initialization).

use zuihitsu::{
    ManualClock, MemoryStore, PromptTemplateName, SeedSelf, Seq, Settings, Store, Timestamp,
    event::EventPayload,
    genesis::{self, GenesisStatus, Rollout},
};

fn seed() -> SeedSelf {
    SeedSelf {
        agent_name: "Kestrel".to_owned(),
        persona: "A thoughtful, discreet companion with a long memory.".to_owned(),
        seed_entries: vec!["I keep what people tell me in confidence.".to_owned()],
    }
}

fn clock() -> ManualClock {
    ManualClock::new(Timestamp::from_millis(1_000))
}

#[test]
fn empty_log_is_empty_status() {
    let store = MemoryStore::new();
    assert_eq!(genesis::status(&store).unwrap(), GenesisStatus::Empty);
}

#[test]
fn rollout_creates_a_complete_agent() {
    let mut store = MemoryStore::new();
    let outcome = genesis::rollout(&mut store, &clock(), &seed()).unwrap();
    assert!(matches!(outcome, Rollout::Created { .. }));
    assert_eq!(genesis::status(&store).unwrap(), GenesisStatus::Complete);

    let events = store.read_from(Seq::ZERO).unwrap();

    // The self memory and its seed disposition entry are present.
    let self_created = events.iter().any(|e| {
        matches!(&e.payload, EventPayload::MemoryCreated { name, .. } if name.as_str() == "self")
    });
    assert!(self_created);
    let seed_entry = events
        .iter()
        .any(|e| matches!(&e.payload, EventPayload::MemoryContentAppended { .. }));
    assert!(seed_entry);

    // The four templates and the same_as seed relation are registered.
    let templates = events
        .iter()
        .filter(|e| matches!(e.payload, EventPayload::PromptTemplateRegistered { .. }))
        .count();
    assert_eq!(templates, 4);
    let same_as = events.iter().any(|e| {
        matches!(&e.payload, EventPayload::LinkTypeRegistered { name, .. } if name.as_str() == "same_as")
    });
    assert!(same_as);
    // The system `confidential` tag is seeded, so a context can be marked confidential.
    let confidential = events.iter().any(|e| {
        matches!(&e.payload, EventPayload::TagCreated { name, .. } if name.as_str() == "confidential")
    });
    assert!(confidential);

    // GenesisCompleted is last, and genesis seeds no created_by link or facts about anyone.
    assert!(matches!(
        events.last().unwrap().payload,
        EventPayload::GenesisCompleted { .. }
    ));
    let any_link = events
        .iter()
        .any(|e| matches!(e.payload, EventPayload::LinkCreated { .. }));
    assert!(!any_link);
}

#[test]
fn rollout_is_idempotent_when_complete() {
    let mut store = MemoryStore::new();
    genesis::rollout(&mut store, &clock(), &seed()).unwrap();
    let head_after_first = store.head().unwrap();

    let outcome = genesis::rollout(&mut store, &clock(), &seed()).unwrap();
    assert_eq!(outcome, Rollout::AlreadyComplete);
    assert_eq!(store.head().unwrap(), head_after_first); // nothing appended
}

#[test]
fn interrupted_genesis_resumes_emitting_only_the_missing() {
    // Simulate a partial genesis: a couple of events landed, but no GenesisCompleted.
    let mut store = MemoryStore::new();
    store
        .append(
            Timestamp::from_millis(500),
            vec![
                EventPayload::PromptTemplateRegistered {
                    name: PromptTemplateName::Scaffold,
                    version: 1,
                    body: "<draft system-prompt scaffold — see docs/spec.md §System prompt>"
                        .to_owned(),
                    source: zuihitsu::EventSource::Orchestration,
                },
                EventPayload::PromptTemplateRegistered {
                    name: PromptTemplateName::DescriptionRegen,
                    version: 1,
                    body: "<draft description-regeneration template>".to_owned(),
                    source: zuihitsu::EventSource::Orchestration,
                },
            ],
        )
        .unwrap();
    assert_eq!(genesis::status(&store).unwrap(), GenesisStatus::Incomplete);
    let head_before = store.head().unwrap();

    let Rollout::Created { events_emitted } =
        genesis::rollout(&mut store, &clock(), &seed()).unwrap()
    else {
        panic!("expected a resuming rollout");
    };

    // The two already-present templates were not re-emitted.
    assert_eq!(genesis::status(&store).unwrap(), GenesisStatus::Complete);
    let total = store.head().unwrap().0 - head_before.0;
    assert_eq!(total as usize, events_emitted);

    // Exactly one copy of each template survives (no duplicates from the resume).
    let events = store.read_from(Seq::ZERO).unwrap();
    let scaffold = events
        .iter()
        .filter(|e| {
            matches!(&e.payload, EventPayload::PromptTemplateRegistered { name, .. } if *name == PromptTemplateName::Scaffold)
        })
        .count();
    assert_eq!(scaffold, 1);
}

#[test]
fn manifest_hash_is_stable_across_a_resume() {
    // A complete genesis and a resumed one over the same seed agree on the manifest hash, since it
    // is computed over content, not minted ids.
    let mut fresh = MemoryStore::new();
    genesis::rollout(&mut fresh, &clock(), &seed()).unwrap();

    let mut resumed = MemoryStore::new();
    resumed
        .append(
            Timestamp::from_millis(500),
            vec![EventPayload::ConfigSet {
                settings: Settings::default(),
                source: zuihitsu::EventSource::Bootstrap,
            }],
        )
        .unwrap();
    genesis::rollout(&mut resumed, &clock(), &seed()).unwrap();

    assert_eq!(genesis_hash(&fresh), genesis_hash(&resumed));
}

fn genesis_hash(store: &MemoryStore) -> String {
    store
        .read_from(Seq::ZERO)
        .unwrap()
        .into_iter()
        .find_map(|e| match e.payload {
            EventPayload::GenesisCompleted { manifest_hash, .. } => Some(manifest_hash),
            _ => None,
        })
        .expect("genesis completed")
}
