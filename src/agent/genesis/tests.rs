//! A fresh log rolls out a complete agent, an interrupted one resumes by emitting only what's
//! missing, and a complete one is left alone — all keyed on the presence of `GenesisCompleted`,
//! never log emptiness (spec §Initialization).
use std::collections::BTreeSet;

use crate::{
    InstanceFeatures,
    agent::genesis::{self, GenesisStatus, Rollout, SeedSelf},
    clock::ManualClock,
    event::{EventPayload, EventSource, PromptTemplateName},
    ids::Seq,
    settings::Settings,
    store::{MemoryStore, Store},
    time::Timestamp,
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

/// The `token_budget` in the `ConfigSet` genesis wrote.
fn logged_token_budget(store: &dyn Store) -> i64 {
    store
        .read_from(Seq::ZERO)
        .unwrap()
        .into_iter()
        .find_map(|event| match event.payload {
            EventPayload::ConfigSet { settings, .. } => Some(settings.compaction.token_budget),
            _ => None,
        })
        .expect("genesis writes a ConfigSet")
}

#[test]
fn the_compaction_budget_is_derived_from_the_context_window() {
    // With a model's window, the initial compaction budget is a fraction of it.
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

    // Without one (an in-memory or model-less instance), the built-in default stands.
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
}

#[test]
fn empty_log_is_empty_status() {
    let store = MemoryStore::new();
    assert_eq!(genesis::status(&store).unwrap(), GenesisStatus::Empty);
}

#[test]
fn genesis_boundary_types_round_trip_as_json() {
    // These cross the HTTP API: `SeedSelf` as the create request, `GenesisStatus`/`Rollout` as
    // responses — so they must survive a JSON round-trip.
    let seed = seed();
    let back: SeedSelf = serde_json::from_str(&serde_json::to_string(&seed).unwrap()).unwrap();
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

    // The self memory and its seed disposition entry are present.
    let self_created = events.iter().any(|e| {
            matches!(&e.payload, EventPayload::MemoryCreated { name, .. } if name.as_str() == "self")
        });
    assert!(self_created);
    let seed_entry = events
        .iter()
        .any(|e| matches!(&e.payload, EventPayload::MemoryContentAppended { .. }));
    assert!(seed_entry);

    // The seven templates and the same_as seed relation are registered.
    let templates = events
        .iter()
        .filter(|e| matches!(e.payload, EventPayload::PromptTemplateRegistered { .. }))
        .count();
    assert_eq!(templates, 7);
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
fn genesis_seeds_the_part_of_membership_relation() {
    // `part_of`/`contains` is the membership-or-aboutness relation: an event, entry-bearing
    // memory, or sub-topic belonging to a topic. Seeding it meets the agent where it already
    // reaches (part_of/contains were the coined names) instead of leaving it to improvise or
    // stretch created_by.
    let mut store = MemoryStore::new();
    genesis::rollout(
        &mut store,
        &clock(),
        &seed(),
        None,
        &InstanceFeatures::default(),
    )
    .unwrap();
    let events = store.read_from(Seq::ZERO).unwrap();

    let part_of = events.iter().find_map(|e| match &e.payload {
        EventPayload::LinkTypeRegistered { name, inverse, .. } if name.as_str() == "part_of" => {
            Some(inverse.as_str().to_owned())
        }
        _ => None,
    });
    assert_eq!(
        part_of.as_deref(),
        Some("contains"),
        "genesis must seed part_of with its contains inverse"
    );
}

#[test]
fn genesis_does_not_seed_learned_social_relations() {
    // Mentorship and venue semantics are the agent's to coin for its own environment, not seeded
    // universals — so genesis must register neither. A minimal-seed instance leaves `mentors` and
    // `located_at` to `links.register`, and a run that reaches for them coins its own.
    let mut store = MemoryStore::new();
    genesis::rollout(
        &mut store,
        &clock(),
        &seed(),
        None,
        &InstanceFeatures::default(),
    )
    .unwrap();
    let events = store.read_from(Seq::ZERO).unwrap();

    let seeded = |forward: &str| {
        events.iter().any(|e| {
                matches!(&e.payload, EventPayload::LinkTypeRegistered { name, .. } if name.as_str() == forward)
            })
    };
    assert!(
        !seeded("mentors"),
        "mentorship is learned, not seeded — genesis must not register mentors"
    );
    assert!(
        !seeded("located_at"),
        "venue semantics are learned, not seeded — genesis must not register located_at"
    );
}

#[test]
fn every_reference_link_example_is_a_seeded_relation() {
    // The drift guard: the link/outgoing/incoming/unlink reference entries illustrate their
    // `relation` argument with example labels. Every such example must name a seeded relation (or
    // its inverse), so an agent copying the example verbatim never hits an "unknown relation"
    // crash. `links.register` is excluded — its example is deliberately an *unseeded* relation,
    // since registering a new one is the whole point of the call.
    let seeded: BTreeSet<String> = super::seed_relations()
        .into_iter()
        .flat_map(|relation| {
            [
                relation.name.as_str().to_owned(),
                relation.inverse.as_str().to_owned(),
            ]
        })
        .collect();

    let reference = crate::agent::lua::api_reference(&InstanceFeatures::default());
    let link_entries = [
        "<memory>:link",
        "<memory>:unlink",
        "<memory>:outgoing",
        "<memory>:incoming",
    ];
    for entry in reference
        .iter()
        .filter(|entry| link_entries.contains(&entry.call.as_str()))
    {
        let text = std::iter::once(entry.doc.as_str())
            .chain(entry.params.iter().map(|param| param.doc.as_str()))
            .collect::<Vec<_>>()
            .join(" ");
        for example in relation_examples(&text) {
            assert!(
                seeded.contains(&example),
                "{}: example relation {example:?} is not a seeded relation — a copied example \
                     would crash with \"unknown relation\"",
                entry.call
            );
        }
    }
}

/// Extract the relation labels a reference entry illustrates: the value after an `e.g. "…"`
/// marker, and any label passed to a `:link("…")` / `:outgoing("…")` / `:incoming("…")` /
/// `:unlink("…")` call form in the prose. Every one must resolve to a seeded relation.
fn relation_examples(text: &str) -> Vec<String> {
    let markers = [
        "e.g. \"",
        ":link(\"",
        ":outgoing(\"",
        ":incoming(\"",
        ":unlink(\"",
    ];
    let mut examples = Vec::new();
    for marker in markers {
        let mut rest = text;
        while let Some(start) = rest.find(marker) {
            let after = &rest[start + marker.len()..];
            if let Some(end) = after.find('"') {
                examples.push(after[..end].to_owned());
                rest = &after[end..];
            } else {
                break;
            }
        }
    }
    examples
}

#[test]
fn the_temporal_extraction_template_teaches_the_anchor_rule() {
    // The temporal-extraction pass must not resolve anaphora against the clock: a phrase whose
    // referent is another stated date or event ("that weekend") is anchored to THAT date, and when
    // nothing anchors it the statement is left unextracted — a fabricated now-relative date reads
    // back as fact and is worse than no date. The body is v2 so a v1 `produced_by` keeps naming
    // the body v1 was generated under.
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

    assert_eq!(version, 2, "the anchor-rule body is registered at v2");
    // The utterance-anchored cases are still resolved against the current time.
    assert!(body.contains("anchored to the moment of speaking"));
    // Anaphora pointing at a sibling statement's date is anchored to THAT date, never the clock.
    assert!(body.contains("that weekend"));
    assert!(body.contains("anchored to THAT date, never to the current time"));
    // When nothing anchors it, the statement is left unextracted rather than clock-resolved.
    assert!(body.contains("leave the statement unextracted"));
    // The stated principle: a wrong now-relative date is worse than no date.
    assert!(body.contains("a fabricated now-relative date is worse than no date"));
    // The `before_after` form is offered for a nameable anchoring memory (the schema supports it).
    assert!(body.contains("`before_after` relative to the anchoring memory"));
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
                EventPayload::prompt_template_registered(
                    // The current Scaffold version, so the idempotent rollout recognizes it as
                    // already present and does not re-emit it.
                    PromptTemplateName::Scaffold,
                    8,
                    "<draft system-prompt scaffold — see docs/spec.md §System prompt>".to_owned(),
                    EventSource::Orchestration,
                ),
                EventPayload::prompt_template_registered(
                    PromptTemplateName::DescriptionRegen,
                    1,
                    "<draft description-regeneration template>",
                    EventSource::Orchestration,
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
    // A complete genesis and a resumed one over the same seed agree on the manifest hash, since
    // it is computed over content, not minted ids.
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
            vec![EventPayload::config_set(
                Settings::default(),
                EventSource::Bootstrap,
            )],
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

/// The Scaffold template body `default_templates` bakes for a feature set — the third gate the
/// transcripts feature must move in lockstep with (Lua registration and the API reference are the
/// other two).
fn scaffold_body(features: &InstanceFeatures) -> String {
    super::default_templates(features)
        .into_iter()
        .find(|template| template.name == PromptTemplateName::Scaffold)
        .map(|template| template.body)
        .expect("default_templates includes the scaffold")
}

#[test]
fn the_scaffold_and_flush_name_the_sandbox_language_as_luau() {
    // The agent-facing prompt surfaces own the Luau identity: the scaffold preamble and the flush
    // template both name Luau (not Lua) as the language emitted through run_lua, and each is bumped
    // to the version that introduced the rename so an older `produced_by` keeps naming its body.
    let templates = super::default_templates(&InstanceFeatures::default());
    let template = |name| {
        templates
            .iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("default_templates includes {name:?}"))
    };

    let scaffold = template(PromptTemplateName::Scaffold);
    assert_eq!(
        scaffold.version, 8,
        "the scaffold is registered at v7 (v6 added the record-or-plain-words branch; v7 threads \
         <memory>:details() and memory.list into the recall and deduplication points)"
    );
    assert!(
        scaffold
            .body
            .contains("emitting Luau through the run_lua tool")
    );
    assert!(!scaffold.body.contains("emitting Lua through"));

    let flush = template(PromptTemplateName::Flush);
    assert_eq!(
        flush.version, 3,
        "the Luau-naming flush is registered at v3"
    );
    assert!(
        flush
            .body
            .contains("emitting Luau through the run_lua tool")
    );
    assert!(!flush.body.contains("emitting Lua through"));
}

#[test]
fn the_calendar_dotpoint_demonstrates_interpolation_not_concatenation() {
    // The calendar-date dotpoint's reminder example uses a backtick interpolation, so the agent
    // adopts the idiom from the example rather than the `..` concatenation it replaced.
    let scaffold = scaffold_body(&InstanceFeatures::default());
    assert!(scaffold.contains("`Reminder for {calendar.next(\"friday\")}`"));
    assert!(!scaffold.contains("\"Reminder for \" .. calendar.next"));
}

#[test]
fn the_transcripts_dotpoint_is_gated_on_the_feature() {
    // On by default, the scaffold teaches convo.turn; disabled, the dotpoint is dropped so the
    // prompt never teaches a practice the runtime rejects.
    assert!(scaffold_body(&InstanceFeatures::default()).contains("convo.turn"));
    let disabled = InstanceFeatures {
        transcripts: false,
        ..Default::default()
    };
    assert!(!scaffold_body(&disabled).contains("convo.turn"));
}

#[test]
fn the_transcripts_dotpoint_teaches_only_the_token_not_a_console_url() {
    // The connector contract: a console URL never reaches the agent, so the scaffold's
    // agent-facing surface teaches only the `[turn:<id>]` token and never mentions a console link
    // or the `?turn=` query form.
    let scaffold = scaffold_body(&InstanceFeatures::default());
    assert!(scaffold.contains("[turn:<id>] token"));
    assert!(!scaffold.contains("console link"));
    assert!(!scaffold.contains("?turn="));
}

#[test]
fn the_recall_hub_walk_clause_is_gated_on_linking() {
    // The recall dotpoint's hub-walk clause leans on link-following (the `linking` feature), so it
    // is present by default and dropped when linking is off — the prompt never teaches a disabled
    // API. The date-fidelity clause is not link-gated and stays either way.
    let on = scaffold_body(&InstanceFeatures::default());
    assert!(on.contains("follow the links the handle shows"));
    assert!(on.contains("occurred_at as it reads back"));

    let disabled = InstanceFeatures {
        linking: false,
        ..Default::default()
    };
    let off = scaffold_body(&disabled);
    assert!(!off.contains("follow the links the handle shows"));
    assert!(off.contains("occurred_at as it reads back"));
}

#[test]
fn the_scaffold_teaches_category_free_withholding() {
    // The impersonation guard is reinforced by a category-free withholding point: a public fact
    // recited back is not identity, and a refusal must not name the withheld category or confirm
    // that something exists — while still sharing what is public. Always-on (it gates on no
    // feature), so it stands under the default and a stripped feature set alike.
    let full = scaffold_body(&InstanceFeatures::default());
    assert!(full.contains("Knowing a public fact is not being someone"));
    assert!(full.contains("withhold without naming what you withhold"));

    let stripped = scaffold_body(&InstanceFeatures {
        linking: false,
        tagging: false,
        merging: false,
        calendar: false,
        transcripts: false,
        ..Default::default()
    });
    assert!(stripped.contains("Knowing a public fact is not being someone"));
}

#[test]
fn the_scaffold_teaches_milestone_decomposition_for_dated_plans() {
    // The event dotpoint teaches that a plan spanning several dated milestones is several dated
    // facts, each recorded under its own occurred_at rather than bundled into one entry stamped
    // with the first date — so every milestone's date stays independently addressable at recall
    // (the checkpoint-recap miss, where a bundled entry dropped the later milestones' dates).
    // It rides the calendar-gated event dotpoint, so it is present by default and dropped when
    // the calendar is off.
    let on = scaffold_body(&InstanceFeatures::default());
    assert!(on.contains("several dated facts, not one"));
    assert!(on.contains("under its own occurred_at"));
    assert!(on.contains("independently addressable"));

    let no_calendar = InstanceFeatures {
        calendar: false,
        ..Default::default()
    };
    assert!(!scaffold_body(&no_calendar).contains("several dated facts, not one"));
}

#[test]
fn the_scaffold_teaches_search_before_creating() {
    // The reuse dotpoint teaches informed creation: search a non-person thing by meaning, or list the
    // stem to see which handles already exist, and reuse what is found rather than guessing a fresh
    // handle, since a guessed name that misses the existing memory mints a duplicate and splits one
    // referent's facts across variants. Always-on (it gates on no feature), so it stands under the
    // default and a stripped feature set alike.
    let full = scaffold_body(&InstanceFeatures::default());
    assert!(full.contains("memory.search by meaning, or memory.list the stem"));
    assert!(full.contains("a guessed name that misses the existing memory mints a second"));

    let stripped = scaffold_body(&InstanceFeatures {
        linking: false,
        tagging: false,
        merging: false,
        calendar: false,
        transcripts: false,
        ..Default::default()
    });
    assert!(stripped.contains("memory.search by meaning, or memory.list the stem"));
}

#[test]
fn the_event_dotpoint_teaches_a_recurring_event_is_one_memory() {
    // The undated-name teaching now covers recurring events explicitly: a repeating gathering is
    // one memory under its generic name, each occurrence dated on its entries — never a date-stamped
    // clone per mention, which splits one gathering's record across variants. It rides the calendar-gated event
    // dotpoint, so it is present by default and dropped when the calendar is off.
    let on = scaffold_body(&InstanceFeatures::default());
    assert!(on.contains("A recurring or repeating gathering is ONE memory under its generic name"));
    assert!(on.contains("never a month- or date-stamped clone"));

    let no_calendar = InstanceFeatures {
        calendar: false,
        ..Default::default()
    };
    assert!(
        !scaffold_body(&no_calendar)
            .contains("A recurring or repeating gathering is ONE memory under its generic name")
    );
}

#[test]
fn the_scaffold_teaches_belief_arbitration() {
    // The conflicting-accounts point is reinforced by a belief-arbitration point: the arbitration
    // record is the turn-end synthesis's to draw from two standing public entries, so the agent
    // must not smooth the pair into one or supersede one with the other on its own authority.
    // Always-on (it gates on no feature).
    let full = scaffold_body(&InstanceFeatures::default());
    assert!(full.contains("The record that the two accounts conflict is not yours to compose"));
    assert!(full.contains("never supersede one with the other on your own authority"));

    let stripped = scaffold_body(&InstanceFeatures {
        linking: false,
        tagging: false,
        merging: false,
        calendar: false,
        transcripts: false,
        ..Default::default()
    });
    assert!(stripped.contains("The record that the two accounts conflict is not yours to compose"));
}

#[test]
fn the_scaffold_teaches_commit_honesty() {
    // The commit-honesty point: a reply may only claim what the commit summary confirms — a block
    // that crashed, came back empty, or ran a revise loop that matched nothing wrote nothing, so
    // the reply must not confirm a write that never landed. Always-on (it gates on no feature), so
    // it stands under the default and a stripped feature set alike.
    let full = scaffold_body(&InstanceFeatures::default());
    assert!(full.contains("Your reply may only claim what the commit summary shows"));
    assert!(full.contains("never confirm a write that never"));

    let stripped = scaffold_body(&InstanceFeatures {
        linking: false,
        tagging: false,
        merging: false,
        calendar: false,
        transcripts: false,
        ..Default::default()
    });
    assert!(stripped.contains("Your reply may only claim what the commit summary shows"));
}

#[test]
fn the_recall_point_teaches_not_to_repeat_a_search() {
    // The recall dotpoint teaches that re-issuing an identical search returns the same hits, so a
    // search that came up short is changed or read, not run again unchanged (the max-steps death
    // where the agent burned its budget on eleven identical searches). The recall point is
    // always-on, so the clause stands under the default and a stripped feature set alike.
    let full = scaffold_body(&InstanceFeatures::default());
    assert!(full.contains("re-issuing the same search returns the same hits"));

    let stripped = scaffold_body(&InstanceFeatures {
        linking: false,
        tagging: false,
        merging: false,
        calendar: false,
        transcripts: false,
        ..Default::default()
    });
    assert!(stripped.contains("re-issuing the same search returns the same hits"));
}

#[test]
fn the_merge_dotpoint_teaches_recording_and_a_rationale_before_proposing() {
    // The merge dotpoint teaches that the adjudicator weighs the recorded entries plus the stated
    // grounds — so record what a person tells you on their current stub before proposing, and
    // state why they match. (The `{ rationale = "…" }` option walkthrough moved to the
    // propose_merge reference entry; the scaffold teaches the principle.) Gated on `merging`:
    // dropped when merging is off, since the prompt must not teach a call the agent cannot make.
    let on = scaffold_body(&InstanceFeatures::default());
    assert!(on.contains("on their current stub before you propose"));
    assert!(on.contains("state your grounds"));

    let off = scaffold_body(&InstanceFeatures {
        merging: false,
        ..Default::default()
    });
    assert!(!off.contains("on their current stub before you propose"));
}
