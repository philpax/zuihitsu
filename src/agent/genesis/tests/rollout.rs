//! Rollout, idempotence, and resume tests.

use crate::{
    InstanceFeatures,
    agent::genesis::{self, GenesisStatus, Rollout},
    event::{EventPayload, EventSource, PromptTemplateName},
    ids::Seq,
    settings::Settings,
    store::{MemoryStore, Store},
    time::Timestamp,
};

use super::{clock, seed};

/// The compaction settings in the `ConfigSet` genesis wrote.
fn logged_compaction(store: &dyn Store) -> crate::settings::CompactionSettings {
    store
        .read_from(Seq::ZERO)
        .unwrap()
        .into_iter()
        .find_map(|event| match event.payload {
            EventPayload::ConfigSet { settings, .. } => Some(settings.compaction),
            _ => None,
        })
        .expect("genesis writes a ConfigSet")
}

/// The `token_budget` in the `ConfigSet` genesis wrote.
fn logged_token_budget(store: &dyn Store) -> i64 {
    logged_compaction(store).token_budget
}

#[test]
fn the_compaction_budget_is_derived_from_the_context_window() {
    let mut store = MemoryStore::new();
    genesis::rollout(
        &mut store,
        &clock(),
        &seed(),
        Some(100_000),
        &InstanceFeatures::default(),
    )
    .unwrap();
    assert_eq!(logged_token_budget(&store), 80_000);
    // The window itself is recorded beside the derived budget, so observers can relate the two.
    assert_eq!(logged_compaction(&store).context_length, Some(100_000));

    let mut store = MemoryStore::new();
    genesis::rollout(
        &mut store,
        &clock(),
        &seed(),
        None,
        &InstanceFeatures::default(),
    )
    .unwrap();
    assert_eq!(
        logged_token_budget(&store),
        Settings::default().compaction.token_budget
    );
    // A model-less instance records no window: unknown, not a fabricated number.
    assert_eq!(logged_compaction(&store).context_length, None);
}

#[test]
fn empty_log_is_empty_status() {
    let store = MemoryStore::new();
    assert_eq!(genesis::status(&store).unwrap(), GenesisStatus::Empty);
}

#[test]
fn genesis_boundary_types_round_trip_as_json() {
    let seed = seed();
    let back: super::SeedSelf =
        serde_json::from_str(&serde_json::to_string(&seed).unwrap()).unwrap();
    assert_eq!(back.agent_name, seed.agent_name);
    assert_eq!(back.seed_entries, seed.seed_entries);
    for status in [
        GenesisStatus::Empty,
        GenesisStatus::Incomplete,
        GenesisStatus::Complete,
    ] {
        assert_eq!(
            serde_json::from_str::<GenesisStatus>(&serde_json::to_string(&status).unwrap())
                .unwrap(),
            status
        );
    }
    let rollout = Rollout::Created { events_emitted: 7 };
    assert_eq!(
        serde_json::from_str::<Rollout>(&serde_json::to_string(&rollout).unwrap()).unwrap(),
        rollout
    );
}

#[test]
fn rollout_creates_a_complete_agent() {
    let mut store = MemoryStore::new();
    let outcome = genesis::rollout(
        &mut store,
        &clock(),
        &seed(),
        None,
        &InstanceFeatures::default(),
    )
    .unwrap();
    assert!(matches!(outcome, Rollout::Created { .. }));
    assert_eq!(genesis::status(&store).unwrap(), GenesisStatus::Complete);

    let events = store.read_from(Seq::ZERO).unwrap();

    let self_created = events.iter().any(|e| {
            matches!(&e.payload, EventPayload::MemoryCreated { name, .. } if name.as_str() == "self")
        });
    assert!(self_created);
    let seed_entry = events
        .iter()
        .any(|e| matches!(&e.payload, EventPayload::MemoryContentAppended { .. }));
    assert!(seed_entry);

    let templates = events
        .iter()
        .filter(|e| matches!(e.payload, EventPayload::PromptTemplateRegistered { .. }))
        .count();
    assert_eq!(templates, 9);
    let same_as = events.iter().any(|e| {
            matches!(&e.payload, EventPayload::LinkTypeRegistered { name, .. } if name.as_str() == "same_as")
        });
    assert!(same_as);
    let confidential = events.iter().any(|e| {
            matches!(&e.payload, EventPayload::TagCreated { name, .. } if name.as_str() == "confidential")
        });
    assert!(confidential);

    assert!(matches!(
        events.last().unwrap().payload,
        EventPayload::GenesisCompleted { .. }
    ));
    let any_link = events
        .iter()
        .any(|e| matches!(e.payload, EventPayload::LinkCreated { .. }));
    assert!(!any_link);

    // Genesis is the one Bootstrap writer: every event it emits carries that authority on the
    // envelope, so a log's birth is distinguishable from everything after it.
    assert!(events.iter().all(|e| e.source == EventSource::Bootstrap));
}

#[test]
fn the_temporal_extraction_template_teaches_the_anchor_rule() {
    let mut store = MemoryStore::new();
    genesis::rollout(
        &mut store,
        &clock(),
        &seed(),
        None,
        &InstanceFeatures::default(),
    )
    .unwrap();
    let (version, body) = store
        .read_from(Seq::ZERO)
        .unwrap()
        .into_iter()
        .find_map(|event| match event.payload {
            EventPayload::PromptTemplateRegistered {
                name: PromptTemplateName::TemporalExtraction,
                version,
                body,
                ..
            } => Some((version, body)),
            _ => None,
        })
        .expect("genesis registers a TemporalExtraction template");

    assert_eq!(
        version, 6,
        "the third-party-routine body is registered at v6"
    );
    assert!(body.contains("The default is to extract nothing"));
    assert!(body.contains("anchored to the moment of speaking"));
    assert!(body.contains("that weekend"));
    assert!(body.contains("resolves against that anchor"));
    // A back-pointing phrase takes the sibling's stated occurrence from its bracket, and one whose
    // anchor cannot be found is omitted rather than defaulted to now.
    assert!(body.contains("· occurred YYYY-MM-DD"));
    assert!(body.contains("takes that statement's occurrence"));
    assert!(body.contains("a phrase whose anchor you cannot find is omitted"));
    assert!(body.contains(
        "Never resolve against the current time a phrase that is not anchored to the moment \
         of speaking"
    ));
    assert!(body.contains("never assigned the current day"));
    assert!(body.contains("worse than no date"));
    // A date modifying a different referent than the statement's subject (an inspiration, a
    // namesake, a comparison) is that referent's date, not the subject's occurrence — omitted.
    assert!(body.contains("describe the statement's own subject"));
    assert!(body.contains("an inspiration, a namesake, a comparison, or a historical analogy"));
    assert!(body.contains("never merely a date the statement mentions"));
    // A recurrence describing another system's or person's own routine dates that mechanism, not the
    // statement, so it is omitted; only a recurrence the agent itself should track is emitted.
    assert!(body.contains("dates the statement only when it is the agent's own to track"));
    assert!(body.contains("their cron job, their nightly rebuild, their weekly publication"));
    assert!(body.contains("Only a recurrence the agent itself should keep"));
    assert!(body.contains("`before_after` relative to another memory named as its"));
}

#[test]
fn rollout_is_idempotent_when_complete() {
    let mut store = MemoryStore::new();
    genesis::rollout(
        &mut store,
        &clock(),
        &seed(),
        None,
        &InstanceFeatures::default(),
    )
    .unwrap();
    let head_after_first = store.head().unwrap();

    let outcome = genesis::rollout(
        &mut store,
        &clock(),
        &seed(),
        None,
        &InstanceFeatures::default(),
    )
    .unwrap();
    assert_eq!(outcome, Rollout::AlreadyComplete);
    assert_eq!(store.head().unwrap(), head_after_first);
}

#[test]
fn interrupted_genesis_resumes_emitting_only_the_missing() {
    let mut store = MemoryStore::new();
    store
        .append(
            Timestamp::from_millis(500),
            EventSource::Agent,
            vec![
                EventPayload::prompt_template_registered(
                    PromptTemplateName::Scaffold,
                    20,
                    "<draft system-prompt scaffold — see docs/conversations-and-briefs.md §System prompt>".to_owned(),
                ),
                EventPayload::prompt_template_registered(
                    PromptTemplateName::DescriptionRegen,
                    1,
                    "<draft description-regeneration template>",
                ),
            ],
        )
        .unwrap();
    assert_eq!(genesis::status(&store).unwrap(), GenesisStatus::Incomplete);
    let head_before = store.head().unwrap();

    let Rollout::Created { events_emitted } = genesis::rollout(
        &mut store,
        &clock(),
        &seed(),
        None,
        &InstanceFeatures::default(),
    )
    .unwrap() else {
        panic!("expected a resuming rollout");
    };

    assert_eq!(genesis::status(&store).unwrap(), GenesisStatus::Complete);
    let total = store.head().unwrap().0 - head_before.0;
    assert_eq!(total as usize, events_emitted);

    let events = store.read_from(Seq::ZERO).unwrap();
    let scaffold = events
            .iter()
            .filter(|e| {
                matches!(&e.payload, EventPayload::PromptTemplateRegistered { name, .. } if *name == PromptTemplateName::Scaffold)
            })
            .count();
    assert_eq!(scaffold, 2);
}

#[test]
fn manifest_hash_is_stable_across_a_resume() {
    let mut fresh = MemoryStore::new();
    genesis::rollout(
        &mut fresh,
        &clock(),
        &seed(),
        None,
        &InstanceFeatures::default(),
    )
    .unwrap();

    let mut resumed = MemoryStore::new();
    resumed
        .append(
            Timestamp::from_millis(500),
            EventSource::Agent,
            vec![EventPayload::config_set(Settings::default())],
        )
        .unwrap();
    genesis::rollout(
        &mut resumed,
        &clock(),
        &seed(),
        None,
        &InstanceFeatures::default(),
    )
    .unwrap();

    assert_eq!(super::genesis_hash(&fresh), super::genesis_hash(&resumed));
}

#[test]
fn reconcile_registers_only_names_the_log_has_never_seen() {
    let mut store = MemoryStore::new();
    let features = InstanceFeatures::default();
    genesis::rollout(&mut store, &clock(), &seed(), None, &features).unwrap();

    // A born log holds every default, so the reconcile has nothing to add.
    let registered = genesis::reconcile_new_templates(&mut store, &clock(), &features).unwrap();
    assert_eq!(
        registered, 0,
        "a log born under this build already holds every default template"
    );
}

#[test]
fn reconcile_backfills_a_template_the_genesis_predates() {
    let mut store = MemoryStore::new();
    let features = InstanceFeatures::default();
    genesis::rollout(&mut store, &clock(), &seed(), None, &features).unwrap();

    // Simulate a log born before a template existed: rebuild the log without any registration of
    // one name, keeping everything else (a MemoryStore replays from events, so the surgery is a
    // filtered copy).
    let dropped = PromptTemplateName::EntryConsolidation;
    let mut old_log = MemoryStore::new();
    for event in store.read_from(Seq::ZERO).unwrap() {
        if matches!(
            &event.payload,
            EventPayload::PromptTemplateRegistered { name, .. } if *name == dropped
        ) {
            continue;
        }
        old_log
            .append(event.recorded_at, event.source.clone(), vec![event.payload])
            .unwrap();
    }

    let defaults = crate::agent::genesis::seed::default_templates(&features);
    let expected: Vec<u32> = defaults
        .iter()
        .filter(|template| template.name == dropped)
        .map(|template| template.version)
        .collect();
    assert!(
        !expected.is_empty(),
        "the dropped name must exist among the build defaults"
    );
    let registered = genesis::reconcile_new_templates(&mut old_log, &clock(), &features).unwrap();
    assert_eq!(
        registered,
        expected.len(),
        "every default version of the absent name is backfilled, and nothing else"
    );
    let template = crate::agent::templates::latest_template(&old_log, dropped)
        .unwrap()
        .expect("the reconcile registered the template");
    assert_eq!(
        Some(template.version),
        expected.iter().copied().max(),
        "the highest default version becomes the latest"
    );

    // Idempotent: a second boot adds nothing.
    let again = genesis::reconcile_new_templates(&mut old_log, &clock(), &features).unwrap();
    assert_eq!(again, 0);
}

#[test]
fn reconcile_never_touches_an_operator_curated_name() {
    let mut store = MemoryStore::new();
    let features = InstanceFeatures::default();
    genesis::rollout(&mut store, &clock(), &seed(), None, &features).unwrap();

    // The operator re-registered a template at a later version; the reconcile must not add a
    // build-default registration beneath it.
    store
        .append(
            Timestamp::from_millis(1),
            EventSource::Operator,
            vec![EventPayload::prompt_template_registered(
                PromptTemplateName::EntryConsolidation,
                7,
                "an operator-authored body".to_owned(),
            )],
        )
        .unwrap();
    let registered = genesis::reconcile_new_templates(&mut store, &clock(), &features).unwrap();
    assert_eq!(registered, 0);
    let template =
        crate::agent::templates::latest_template(&store, PromptTemplateName::EntryConsolidation)
            .unwrap()
            .expect("the operator's registration stands");
    assert_eq!(
        template.version, 7,
        "the operator's version stays the latest"
    );
}
