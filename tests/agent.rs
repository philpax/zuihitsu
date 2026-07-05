//! Agent-loop tests: a scripted model drives the step loop through tool calls and terminals, and
//! the resulting turns and side effects land in the log (spec §Agent loop).

mod common;

use serde::Serialize;

use common::Harness;
use zuihitsu::{
    BlockOutcome, CaptureLevel, CivilDate, Completion, EntryId, EnvConfig, Event, EventPayload,
    InferredLink, InstanceFeatures, LinkInferenceArgs, Message, ModelPhase, Namespace,
    NewRelationSpec, OpenAiClient, PromptTemplateName, RequestRecord, ScriptedModel, SeedSelf, Seq,
    TerminalCause, Timestamp, ToolCall, ToolChoice, TurnOutcome, TurnReport, TurnRole, Usage,
    buffer_turns, genesis, run_turn,
};

fn seed() -> SeedSelf {
    SeedSelf {
        agent_name: "Kestrel".to_owned(),
        persona: "A companion.".to_owned(),
        seed_entries: Vec::new(),
    }
}

fn run_lua_call(script: &str) -> Completion {
    Completion::ToolCalls(vec![ToolCall {
        id: "1".to_owned(),
        name: "run_lua".to_owned(),
        arguments: serde_json::json!({ "script": common::prepare_script(script) }).to_string(),
    }])
}

fn count_agent_turns(events: &[Event]) -> usize {
    events
        .iter()
        .filter(|e| {
            matches!(
                &e.payload,
                EventPayload::ConversationTurn {
                    role: TurnRole::Agent,
                    ..
                }
            )
        })
        .count()
}

#[tokio::test]
async fn tool_call_then_reply_commits_and_replies() {
    let h = Harness::new();
    let model = ScriptedModel::new([
        run_lua_call(r#"memory.create(PERSON_DAVE, "Met at the climbing gym")"#),
        Completion::Reply("Noted — I'll remember Dave.".to_owned()),
    ]);

    let TurnReport { outcome, .. } = run_turn(h.as_turn(&model, "Remember Dave", 8))
        .await
        .unwrap();

    assert_eq!(
        outcome,
        TurnOutcome::Reply("Noted — I'll remember Dave.".to_owned())
    );
    // The tool call's side effect committed and projected.
    assert!(
        h.engine
            .graph
            .lock()
            .memory_by_name(Namespace::Person.with_name("dave"))
            .unwrap()
            .is_some()
    );
    // Exactly one agent turn for the cycle, plus the inbound participant turn and a LuaExecuted.
    assert_eq!(count_agent_turns(&h.events()), 1);
    let events = h.events();
    assert!(events.iter().any(|e| matches!(
        &e.payload,
        EventPayload::ConversationTurn {
            role: TurnRole::Participant,
            ..
        }
    )));
    assert!(
        events
            .iter()
            .any(|e| matches!(e.payload, EventPayload::LuaExecuted { .. }))
    );
}

/// The verbatim special-token leak observed in the wild: a pseudo-tool-call the model transcribed as
/// plain reply text at the forced-answer step.
const MALFORMED_REPLY: &str =
    "<|tool_call>call:run_lua{script:<|\"|>memory.search(\"decided\")<|\"|>}<tool_call|>";

/// Whether any recorded `ConversationTurn` carries the special-token markup — the invariant the guard
/// protects: such markup must never reach a persisted turn, and so never a participant.
fn any_turn_contains(events: &[Event], needle: &str) -> bool {
    events.iter().any(|e| {
        matches!(
            &e.payload,
            EventPayload::ConversationTurn { text, .. } if text.contains(needle)
        )
    })
}

#[tokio::test]
async fn a_malformed_reply_is_resampled_and_the_clean_retry_lands() {
    let h = Harness::new();
    // The first completion leaks special-token markup; the guard resamples the same step, and the
    // clean retry is what the participant receives.
    let model = ScriptedModel::new([
        Completion::Reply(MALFORMED_REPLY.to_owned()),
        Completion::Reply("Noted — I'll remember that.".to_owned()),
    ]);

    let TurnReport { outcome, .. } = run_turn(h.as_turn(&model, "What did we decide?", 8))
        .await
        .unwrap();

    assert_eq!(
        outcome,
        TurnOutcome::Reply("Noted — I'll remember that.".to_owned())
    );
    // Exactly one agent turn recorded, and the markup appears in no ConversationTurn.
    assert_eq!(count_agent_turns(&h.events()), 1);
    assert!(!any_turn_contains(&h.events(), "<|"));
    assert!(!any_turn_contains(&h.events(), "|>"));
}

#[tokio::test]
async fn two_consecutive_malformed_replies_fall_to_the_silent_terminal() {
    let h = Harness::new();
    // Both the initial reply and its resample leak markup; the guard delivers silence rather than
    // markup, and the malformed text is recorded nowhere.
    let model = ScriptedModel::new([
        Completion::Reply(MALFORMED_REPLY.to_owned()),
        Completion::Reply(MALFORMED_REPLY.to_owned()),
    ]);

    let TurnReport { outcome, .. } = run_turn(h.as_turn(&model, "What did we decide?", 8))
        .await
        .unwrap();

    assert_eq!(outcome, TurnOutcome::Silent);
    // A single (empty) agent turn stands for the silent terminal, and the markup is nowhere in the log.
    assert_eq!(count_agent_turns(&h.events()), 1);
    assert!(!any_turn_contains(&h.events(), "<|"));
    assert!(!any_turn_contains(&h.events(), "|>"));
}

#[tokio::test]
async fn descriptions_regenerate_after_a_turn() {
    let h = Harness::new();
    // Genesis registers the description-regen template the write path reads.
    genesis::rollout(
        h.engine.store.lock().as_mut(),
        &h.clock,
        &seed(),
        None,
        &InstanceFeatures::default(),
    )
    .unwrap();
    h.engine
        .graph
        .lock()
        .materialize_from(h.engine.store.lock().as_ref())
        .unwrap();
    h.baseline_descriptions();

    let model = ScriptedModel::new([
        // A public fact about Dave (the description is synthesized from Public entries only).
        run_lua_call(
            r#"local d = memory.create(PERSON_DAVE)
               d:append("Met at the climbing gym", { by_agent = true, visibility = "public" })"#,
        ),
        Completion::Reply("Noted — I'll remember Dave.".to_owned()),
        // The post-turn synthesis call: a `response_format`-constrained reply carries the description
        // as clean JSON (the entry has no temporal phrase, so no occurrences).
        synthesize_call(SynthesizeReply::description(
            "Dave, whom I met at the climbing gym.",
        )),
    ]);

    run_turn(h.as_turn(&model, "Remember Dave", 8))
        .await
        .unwrap();
    h.describe(&model).await;

    // The written memory's description was regenerated from its entries after the cycle.
    let dave = h
        .engine
        .graph
        .lock()
        .memory_by_name(Namespace::Person.with_name("dave"))
        .unwrap()
        .unwrap();
    assert_eq!(dave.description, "Dave, whom I met at the climbing gym.");
    // It carries provenance: which model and template produced it. (Genesis also seeds self's
    // description, with null provenance, so match Dave's specifically.)
    let produced_by = h
        .events()
        .into_iter()
        .find_map(|e| match e.payload {
            EventPayload::MemoryDescriptionRegenerated {
                id, produced_by, ..
            } if id == dave.id => Some(produced_by),
            _ => None,
        })
        .expect("Dave's description was regenerated")
        .expect("regeneration records its provenance");
    assert_eq!(produced_by.model_id, "scripted-model");
    assert_eq!(
        produced_by.template_name,
        PromptTemplateName::DescriptionRegen
    );
    assert_eq!(produced_by.template_version, 1);
}

#[tokio::test]
async fn a_rename_re_describes_the_memory_under_the_new_name() {
    // A rename changes no content, but the description is synthesized under the memory's name, so it
    // must be re-synthesized — otherwise the description (which reaches participants in briefs) keeps
    // the old name (spec §Identity → Renaming, deadname-safety).
    let h = Harness::new();
    genesis::rollout(
        h.engine.store.lock().as_mut(),
        &h.clock,
        &seed(),
        None,
        &InstanceFeatures::default(),
    )
    .unwrap();
    h.engine
        .graph
        .lock()
        .materialize_from(h.engine.store.lock().as_ref())
        .unwrap();
    h.baseline_descriptions();

    let model = ScriptedModel::new([
        // Turn 1: a public fact, then its description synthesized under the old name.
        run_lua_call(
            r#"local d = memory.create(PERSON_DAVE)
               d:append("Handles the deploys.", { by_agent = true, visibility = "public" })"#,
        ),
        Completion::Reply("Noted.".to_owned()),
        synthesize_call(SynthesizeReply::description("Dave handles the deploys.")),
        // Turn 2: the rename — no content change.
        run_lua_call(r#"memory.get(PERSON_DAVE):rename(PERSON_SARAH)"#),
        Completion::Reply("Will do.".to_owned()),
        // The rename re-triggers synthesis, now under the new name — no "Dave".
        synthesize_call(SynthesizeReply::description("Sarah handles the deploys.")),
    ]);

    run_turn(h.as_turn(&model, "Dave handles the deploys", 8))
        .await
        .unwrap();
    h.describe(&model).await;
    let dave = h
        .engine
        .graph
        .lock()
        .memory_by_name(Namespace::Person.with_name("dave"))
        .unwrap()
        .unwrap();
    assert_eq!(dave.description, "Dave handles the deploys.");

    run_turn(h.as_turn(&model, "I go by Sarah now", 8))
        .await
        .unwrap();
    h.describe(&model).await;

    // The rename alone re-described the memory under the new handle; the old name is gone from the
    // description, so it no longer rides into a brief.
    let sarah = h
        .engine
        .graph
        .lock()
        .memory_by_name(Namespace::Person.with_name("sarah"))
        .unwrap()
        .unwrap();
    assert_eq!(sarah.description, "Sarah handles the deploys.");
}

/// Day-noon millis for a `YYYY-MM-DD`, the `occurred_sort` a `Day` occurrence denormalizes to.
fn day_noon(date: &str) -> Timestamp {
    let midnight = CivilDate(date.into()).midnight_millis().unwrap();
    Timestamp::from_millis(midnight + 86_400_000 / 2)
}

/// The post-turn synthesis is now a `response_format`-constrained call: the model returns the
/// `SynthesizeArgs` JSON as its reply (the schema may arrive fenced; the parser locates the object), so
/// a scripted synthesis is a `Reply` carrying that JSON rather than a forced tool call.
fn synthesize_call(reply: SynthesizeReply) -> Completion {
    Completion::Reply(serde_json::to_string(&reply).unwrap())
}

/// The description-synthesis reply shape the test scripts as JSON, now typed so a call site reads
/// as what it is rather than a raw string. Mirrors the `SynthesizeArgs` the describe pass sends to
/// the model (see `src/agent/turn/describe.rs`).
#[derive(Debug, Clone, Serialize)]
struct SynthesizeReply {
    description: String,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    occurrences: Vec<SynthesizeOccurrence>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    arbitration: Option<SynthesizeArbitration>,
}

impl SynthesizeReply {
    fn description(text: impl Into<String>) -> Self {
        SynthesizeReply {
            description: text.into(),
            occurrences: Vec::new(),
            arbitration: None,
        }
    }

    fn with_occurrence(mut self, occurrence: SynthesizeOccurrence) -> Self {
        self.occurrences.push(occurrence);
        self
    }

    fn with_arbitration(mut self, arbitration: SynthesizeArbitration) -> Self {
        self.arbitration = Some(arbitration);
        self
    }
}

#[derive(Debug, Clone, Serialize)]
struct SynthesizeOccurrence {
    entry: usize,
    occurred_at: SynthesizeTime,
}

impl SynthesizeOccurrence {
    /// An occurrence on a specific day (the common case in tests).
    fn day(entry: usize, day: impl Into<String>) -> Self {
        SynthesizeOccurrence {
            entry,
            occurred_at: SynthesizeTime::day(day),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
enum SynthesizeTime {
    Day { day: String },
}

impl SynthesizeTime {
    fn day(day: impl Into<String>) -> Self {
        SynthesizeTime::Day { day: day.into() }
    }
}

#[derive(Debug, Clone, Serialize)]
struct SynthesizeArbitration {
    competing: Vec<usize>,
    credited: Vec<usize>,
    statement: String,
}

fn temporal_resolutions(events: &[Event]) -> Vec<EventPayload> {
    events
        .iter()
        .map(|e| &e.payload)
        .filter(|p| matches!(p, EventPayload::EntryTemporalResolved { .. }))
        .cloned()
        .collect()
}

#[tokio::test]
async fn temporal_extraction_resolves_an_untimed_entry() {
    let h = Harness::new();
    genesis::rollout(
        h.engine.store.lock().as_mut(),
        &h.clock,
        &seed(),
        None,
        &InstanceFeatures::default(),
    )
    .unwrap();
    h.engine
        .graph
        .lock()
        .materialize_from(h.engine.store.lock().as_ref())
        .unwrap();
    h.baseline_descriptions();

    let model = ScriptedModel::new([
        run_lua_call(r#"memory.create(PERSON_DAVE, "Met Dave last Tuesday")"#),
        Completion::Reply("Noted.".to_owned()),
        // The synthesis call resolves statement 1's "last Tuesday" to a concrete day.
        synthesize_call(
            SynthesizeReply::description("Dave, met recently.")
                .with_occurrence(SynthesizeOccurrence::day(1, "2026-06-02")),
        ),
    ]);
    run_turn(h.as_turn(&model, "Remember Dave", 8))
        .await
        .unwrap();
    h.describe(&model).await;

    // The untimed entry gained an occurrence, and an EntryTemporalResolved records it.
    let dave = h
        .engine
        .graph
        .lock()
        .memory_by_name(Namespace::Person.with_name("dave"))
        .unwrap()
        .unwrap();
    let entries = h.engine.graph.lock().entries_local(dave.id).unwrap();
    assert_eq!(entries[0].occurred_sort, Some(day_noon("2026-06-02")));
    assert_eq!(temporal_resolutions(&h.events()).len(), 1);
}

#[tokio::test]
async fn temporal_extraction_does_not_override_an_explicit_occurred_at() {
    let h = Harness::new();
    genesis::rollout(
        h.engine.store.lock().as_mut(),
        &h.clock,
        &seed(),
        None,
        &InstanceFeatures::default(),
    )
    .unwrap();
    h.engine
        .graph
        .lock()
        .materialize_from(h.engine.store.lock().as_ref())
        .unwrap();

    let model = ScriptedModel::new([
        run_lua_call(
            r#"local d = memory.create(PERSON_DAVE)
               d:append("Met Dave", { occurred_at = { day = "2020-01-01" }, visibility = "public" })"#,
        ),
        Completion::Reply("Noted.".to_owned()),
        // The model tries to time statement 1, but the agent already set it explicitly.
        synthesize_call(
            SynthesizeReply::description("Dave.")
                .with_occurrence(SynthesizeOccurrence::day(1, "2026-06-02")),
        ),
    ]);
    run_turn(h.as_turn(&model, "Remember Dave", 8))
        .await
        .unwrap();

    // The explicit occurrence stands; extraction emitted nothing for the already-timed entry.
    let dave = h
        .engine
        .graph
        .lock()
        .memory_by_name(Namespace::Person.with_name("dave"))
        .unwrap()
        .unwrap();
    let entries = h.engine.graph.lock().entries_local(dave.id).unwrap();
    assert_eq!(entries[0].occurred_sort, Some(day_noon("2020-01-01")));
    assert!(temporal_resolutions(&h.events()).is_empty());
}

fn belief_arbitrations(events: &[Event]) -> Vec<EventPayload> {
    events
        .iter()
        .map(|e| &e.payload)
        .filter(|p| matches!(p, EventPayload::BeliefArbitrated { .. }))
        .cloned()
        .collect()
}

#[tokio::test]
async fn a_regen_conflict_emits_belief_arbitrated() {
    let h = Harness::new();
    genesis::rollout(
        h.engine.store.lock().as_mut(),
        &h.clock,
        &seed(),
        None,
        &InstanceFeatures::default(),
    )
    .unwrap();
    h.engine
        .graph
        .lock()
        .materialize_from(h.engine.store.lock().as_ref())
        .unwrap();
    h.baseline_descriptions();

    let model = ScriptedModel::new([
        run_lua_call(
            r#"local d = memory.create(PERSON_DAVE)
               d:append("Dave works at Acme", { by_agent = true, visibility = "public" })
               d:append("Dave works at Hooli", { by_agent = true, visibility = "public" })"#,
        ),
        Completion::Reply("Noted.".to_owned()),
        // Statements 1 and 2 conflict; the synthesis credits the second.
        synthesize_call(
            SynthesizeReply::description("Dave works at Hooli.").with_arbitration(
                SynthesizeArbitration {
                    competing: vec![1, 2],
                    credited: vec![2],
                    statement: "Credited the more recent: Dave works at Hooli.".to_owned(),
                },
            ),
        ),
    ]);
    run_turn(h.as_turn(&model, "Where does Dave work?", 8))
        .await
        .unwrap();
    h.describe(&model).await;

    let dave = h
        .engine
        .graph
        .lock()
        .memory_by_name(Namespace::Person.with_name("dave"))
        .unwrap()
        .unwrap();
    let entries = h.engine.graph.lock().entries_local(dave.id).unwrap();
    let arbitrations = belief_arbitrations(&h.events());
    assert_eq!(arbitrations.len(), 1);
    let EventPayload::BeliefArbitrated {
        memory,
        competing_entries,
        resolution,
        produced_by,
    } = &arbitrations[0]
    else {
        unreachable!();
    };
    assert_eq!(*memory, dave.id);
    // The 1-based statement numbers resolved to the two entries' ids, in order.
    assert_eq!(
        *competing_entries,
        vec![entries[0].entry_id, entries[1].entry_id]
    );
    assert_eq!(resolution.credited, vec![entries[1].entry_id]);
    assert!(resolution.statement.contains("Hooli"));
    assert!(produced_by.is_some());
}

#[tokio::test]
async fn a_single_sided_arbitration_is_dropped() {
    let h = Harness::new();
    genesis::rollout(
        h.engine.store.lock().as_mut(),
        &h.clock,
        &seed(),
        None,
        &InstanceFeatures::default(),
    )
    .unwrap();
    h.engine
        .graph
        .lock()
        .materialize_from(h.engine.store.lock().as_ref())
        .unwrap();

    let model = ScriptedModel::new([
        run_lua_call(r#"memory.create(PERSON_DAVE, "Met Dave")"#),
        Completion::Reply("Noted.".to_owned()),
        // Only one "competing" statement — not a real conflict, so nothing is recorded.
        synthesize_call(SynthesizeReply::description("Dave.").with_arbitration(
            SynthesizeArbitration {
                competing: vec![1],
                credited: vec![1],
                statement: "only one side".to_owned(),
            },
        )),
    ]);
    run_turn(h.as_turn(&model, "Remember Dave", 8))
        .await
        .unwrap();

    assert!(belief_arbitrations(&h.events()).is_empty());
}

#[tokio::test]
async fn a_neutral_third_entry_does_not_dilute_the_contradiction() {
    // The conflicting-accounts shape the eval missed in 3 of 5 runs: two accounts of one fact stand
    // as sibling public entries, and a neutral third entry (the event's own title) sits alongside
    // them. The synthesis prompt now closes with an explicit pairwise contradiction check over the
    // numbered statements, so the scripted model pairs statements 2 and 3 while crediting neither,
    // and a `BeliefArbitrated` with an empty `credited` lands. Both the emitted event and the shape
    // of the prompt the pass sent are asserted, since the fix is a prompt change.
    let h = Harness::new();
    genesis::rollout(
        h.engine.store.lock().as_mut(),
        &h.clock,
        &seed(),
        None,
        &InstanceFeatures::default(),
    )
    .unwrap();
    h.engine
        .graph
        .lock()
        .materialize_from(h.engine.store.lock().as_ref())
        .unwrap();
    h.baseline_descriptions();

    // One neutral title entry and two conflicting location accounts (in the live scenario these
    // arrive from two tellers across turns; here the memory's public entries are scripted directly).
    let model = ScriptedModel::new([
        run_lua_call(
            r#"local e = memory.create(EVENT_ALL_HANDS)
               e:append("The all-hands meeting", { by_agent = true, visibility = "public" })
               e:append("Located in the main auditorium", { by_agent = true, visibility = "public" })
               e:append("Located in the rooftop terrace", { by_agent = true, visibility = "public" })"#,
        ),
        Completion::Reply("Noted.".to_owned()),
        // The neutral statement 1 stands apart; statements 2 and 3 collide, and neither is yet known
        // to be right, so `credited` is left empty.
        synthesize_call(
            SynthesizeReply::description(
                "The all-hands meeting, reported in either the main auditorium or the rooftop \
                 terrace — the accounts disagree.",
            )
            .with_arbitration(SynthesizeArbitration {
                competing: vec![2, 3],
                credited: vec![],
                statement: "Two standing accounts of the location: the main auditorium and the \
                            rooftop terrace, neither retracted."
                    .to_owned(),
            }),
        ),
    ]);
    run_turn(h.as_turn(&model, "Where is the all-hands?", 8))
        .await
        .unwrap();
    h.describe(&model).await;

    let all_hands = h
        .engine
        .graph
        .lock()
        .memory_by_name(Namespace::Event.with_name("all-hands"))
        .unwrap()
        .unwrap();
    let entries = h.engine.graph.lock().entries_local(all_hands.id).unwrap();
    let arbitrations = belief_arbitrations(&h.events());
    assert_eq!(arbitrations.len(), 1);
    let EventPayload::BeliefArbitrated {
        memory,
        competing_entries,
        resolution,
        ..
    } = &arbitrations[0]
    else {
        unreachable!();
    };
    assert_eq!(*memory, all_hands.id);
    // The two conflicting statements (2 and 3) resolved to their entries; the neutral first entry is
    // not among them.
    assert_eq!(
        *competing_entries,
        vec![entries[1].entry_id, entries[2].entry_id]
    );
    // Both accounts stand: neither is credited above the other.
    assert!(resolution.credited.is_empty());

    // The pass posed the contradiction question over the numbered statements: the synthesis prompt
    // carries the numbered statements and the explicit pairwise contradiction ask.
    let prompts: Vec<String> = model
        .recorded_messages()
        .iter()
        .flatten()
        .map(|message| message.content.clone())
        .collect();
    assert!(
        prompts.iter().any(|p| {
            p.contains("1. The all-hands meeting")
                && p.contains("2. Located in the main auditorium")
                && p.contains("3. Located in the rooftop terrace")
                && p.contains("incompatible values for the same fact")
        }),
        "the synthesis prompt must number the statements and pose the contradiction check: {prompts:?}"
    );
}

#[tokio::test]
async fn a_private_entry_stays_out_of_the_description_but_is_still_extracted() {
    let h = Harness::new();
    genesis::rollout(
        h.engine.store.lock().as_mut(),
        &h.clock,
        &seed(),
        None,
        &InstanceFeatures::default(),
    )
    .unwrap();
    h.engine
        .graph
        .lock()
        .materialize_from(h.engine.store.lock().as_ref())
        .unwrap();
    h.baseline_descriptions();

    // Dave's memory carries one public fact and one private, future-dated aside.
    let model = ScriptedModel::new([
        run_lua_call(
            r#"local d = memory.create(PERSON_DAVE)
               d:append("Dave is a climber", { by_agent = true, visibility = "public" })
               d:append("Dave has a private therapy session next Tuesday", { by_agent = true, visibility = "private" })"#,
        ),
        Completion::Reply("Noted.".to_owned()),
        // The description pass sees only the public entry.
        synthesize_call(SynthesizeReply::description("Dave is a climber.")),
        // The focused extraction pass over the private untimed entry resolves its occurrence.
        synthesize_call(
            SynthesizeReply::description("(discarded)")
                .with_occurrence(SynthesizeOccurrence::day(1, "2026-06-16")),
        ),
    ]);
    run_turn(h.as_turn(&model, "Remember Dave", 8))
        .await
        .unwrap();
    h.describe(&model).await;

    // The description-synthesis prompt was shown the public fact but never the private aside — the leak
    // the split closes.
    let prompts: Vec<String> = model
        .recorded_messages()
        .iter()
        .flatten()
        .map(|message| message.content.clone())
        .collect();
    assert!(
        prompts
            .iter()
            .any(|p| p.contains("Dave is a climber") && !p.contains("therapy")),
        "the description pass must not see the private entry: {prompts:?}"
    );
    assert!(
        prompts.iter().any(|p| p.contains("therapy")),
        "the private entry must still reach the focused extraction pass"
    );

    // Yet the private entry still gained its occurrence (so a private reminder can still fire).
    let dave = h
        .engine
        .graph
        .lock()
        .memory_by_name(Namespace::Person.with_name("dave"))
        .unwrap()
        .unwrap();
    let entries = h.engine.graph.lock().entries_local(dave.id).unwrap();
    let therapy = entries
        .iter()
        .find(|entry| entry.text.contains("therapy"))
        .unwrap();
    assert_eq!(therapy.occurred_sort, Some(day_noon("2026-06-16")));
}

#[tokio::test]
async fn agent_turns_record_their_provenance() {
    let h = Harness::new();
    // Genesis registers the scaffold the agent turn runs against.
    genesis::rollout(
        h.engine.store.lock().as_mut(),
        &h.clock,
        &seed(),
        None,
        &InstanceFeatures::default(),
    )
    .unwrap();
    h.engine
        .graph
        .lock()
        .materialize_from(h.engine.store.lock().as_ref())
        .unwrap();

    let model = ScriptedModel::new([Completion::Reply("Noted.".to_owned())]);
    run_turn(h.as_turn(&model, "hello", 8)).await.unwrap();

    let turns: Vec<(TurnRole, Option<_>)> = h
        .events()
        .into_iter()
        .filter_map(|e| match e.payload {
            EventPayload::ConversationTurn {
                role, produced_by, ..
            } => Some((role, produced_by)),
            _ => None,
        })
        .collect();

    // The inbound participant turn is not inference, so it has no provenance.
    let (_, participant) = turns
        .iter()
        .find(|(role, _)| *role == TurnRole::Participant)
        .expect("a participant turn");
    assert!(participant.is_none());

    // The agent turn records the chat model and the scaffold it ran against.
    let (_, agent) = turns
        .iter()
        .find(|(role, _)| *role == TurnRole::Agent)
        .expect("an agent turn");
    let provenance = agent
        .as_ref()
        .expect("the agent turn records its provenance");
    assert_eq!(provenance.model_id, "scripted-model");
    assert_eq!(provenance.template_name, PromptTemplateName::Scaffold);
    assert_eq!(provenance.template_version, 3);
}

#[tokio::test]
async fn stay_silent_terminal_posts_nothing() {
    let h = Harness::new();
    let model = ScriptedModel::new([Completion::Silent]);

    let TurnReport { outcome, .. } = run_turn(h.as_turn(&model, "(chatter)", 8)).await.unwrap();

    assert_eq!(outcome, TurnOutcome::Silent);
    // Auditable silence: an agent turn is still recorded, with empty text.
    let silent_recorded = h.events().into_iter().any(|e| {
        matches!(
            &e.payload,
            EventPayload::ConversationTurn { role: TurnRole::Agent, text, .. } if text.is_empty()
        )
    });
    assert!(silent_recorded);
}

#[tokio::test]
async fn max_steps_ends_the_turn_with_a_surfaced_error() {
    let h = Harness::new();
    // A model that only ever calls tools, never terminating.
    let model = ScriptedModel::new([
        run_lua_call("return 1"),
        run_lua_call("return 2"),
        run_lua_call("return 3"),
    ]);

    let TurnReport { outcome, .. } = run_turn(h.as_turn(&model, "loop forever", 2))
        .await
        .unwrap();

    assert_eq!(outcome, TurnOutcome::MaxStepsExceeded);
    // The cycle still records exactly one agent turn, carrying the surfaced error.
    assert_eq!(count_agent_turns(&h.events()), 1);
    let surfaced = h.events().into_iter().any(|e| {
        matches!(
            &e.payload,
            EventPayload::ConversationTurn { role: TurnRole::Agent, text, .. } if text.contains("max steps")
        )
    });
    assert!(surfaced);
}

/// The nearing-budget nudge (a system message telling the model to wrap up) lands exactly once, on
/// the step two before the bound, and persists into the frames after it — so the model gets the
/// legibility warning without it being re-appended every remaining step.
#[tokio::test]
async fn the_nearing_budget_nudge_lands_once_at_max_minus_two() {
    let h = Harness::new();
    // max_steps = 3: the nudge is due before step index 1 (max_steps - 2).
    let model = ScriptedModel::new([
        run_lua_call("return 1"),
        run_lua_call("return 2"),
        Completion::Reply("done".to_owned()),
    ]);

    run_turn(h.as_turn(&model, "go", 3)).await.unwrap();

    let nudge = "two steps remain in this turn";
    let count = |messages: &[Message]| {
        messages
            .iter()
            .filter(|m| m.content.contains(nudge))
            .count()
    };
    let seen = model.recorded_messages();
    assert_eq!(seen.len(), 3, "three generate calls");
    assert_eq!(count(&seen[0]), 0, "no nudge before the max-2 step");
    assert_eq!(count(&seen[1]), 1, "the nudge lands on the max-2 step");
    assert_eq!(count(&seen[2]), 1, "it persists once, not re-appended");
}

/// On the final step the loop withdraws the tools (`ToolChoice::None`) so the model must answer with
/// what it has, and that text terminates the turn as an ordinary `Reply` — not a `MaxStepsExceeded`.
#[tokio::test]
async fn the_final_step_forces_a_textual_answer() {
    let h = Harness::new();
    let model = ScriptedModel::new([
        run_lua_call("return 1"),
        run_lua_call("return 2"),
        Completion::Reply("here is what I found".to_owned()),
    ]);

    let TurnReport { outcome, .. } = run_turn(h.as_turn(&model, "go", 3)).await.unwrap();

    assert_eq!(
        outcome,
        TurnOutcome::Reply("here is what I found".to_owned())
    );
    // The earlier steps let the model choose; only the final step withdraws the tools.
    assert_eq!(
        model.recorded_tool_choices(),
        vec![ToolChoice::Auto, ToolChoice::Auto, ToolChoice::None],
    );
}

/// The forced final answer is a nudge, not a guarantee: a model that still produces no text on the
/// final step (a tool call, defying the withdrawn tools) falls back to the surfaced `MaxStepsExceeded`
/// terminal — the fallback the loop keeps, not the norm.
#[tokio::test]
async fn a_model_that_produces_no_text_on_the_final_step_still_max_steps() {
    let h = Harness::new();
    let model = ScriptedModel::new([run_lua_call("return 1"), run_lua_call("return 2")]);

    let TurnReport { outcome, .. } = run_turn(h.as_turn(&model, "go", 2)).await.unwrap();

    assert_eq!(outcome, TurnOutcome::MaxStepsExceeded);
    assert_eq!(count_agent_turns(&h.events()), 1);
    // The final step still had its tools withdrawn, even though the model did not reply.
    assert_eq!(
        model.recorded_tool_choices().last(),
        Some(&ToolChoice::None)
    );
}

#[tokio::test]
async fn tool_result_feeds_back_across_steps() {
    let h = Harness::new();
    // First create, then a second block reads it back, then reply — exercising multi-step flow.
    let model = ScriptedModel::new([
        run_lua_call(r#"memory.create(TOPIC_CLIMBING, "Bouldering and sport climbing")"#),
        run_lua_call(r#"return memory.get(TOPIC_CLIMBING):entries()"#),
        Completion::Reply("done".to_owned()),
    ]);

    let TurnReport { outcome, .. } = run_turn(h.as_turn(&model, "go", 8)).await.unwrap();
    assert_eq!(outcome, TurnOutcome::Reply("done".to_owned()));

    // Two LuaExecuted events (two blocks), both committed.
    let lua_events = h
        .events()
        .into_iter()
        .filter(|e| matches!(e.payload, EventPayload::LuaExecuted { .. }))
        .count();
    assert_eq!(lua_events, 2);
}

#[tokio::test]
async fn tool_calls_persist_in_the_buffer_across_turns() {
    // A turn's run_lua blocks (script + result) should survive into the next turn's buffer so the
    // model sees what it already did — not just the reply text, but the tool interaction itself.
    let h = Harness::new();
    let model = ScriptedModel::new([
        run_lua_call("return 'hello from lua'"),
        Completion::Reply("done".to_owned()),
        Completion::Reply("ok".to_owned()),
    ]);
    let conversation = h.session.conversation();

    // Turn 1: a run_lua block then a reply.
    run_turn(h.as_turn(&model, "go", 8)).await.unwrap();

    // Rebuild the buffer from the recorded turns.
    let buffer = buffer_turns(h.engine.store.lock().as_ref(), conversation, Seq::ZERO).unwrap();

    // The agent's turn view should carry the tool step.
    let agent_turn = buffer
        .iter()
        .find(|t| t.role == TurnRole::Agent)
        .expect("an agent turn");
    assert_eq!(
        agent_turn.steps.len(),
        1,
        "the agent turn carries its run_lua step"
    );
    assert!(agent_turn.steps[0].result.contains("hello from lua"));

    // Turn 2: the model's first generate call should include the tool-call/result pair in its messages.
    run_turn(h.as_turn_buffered(&model, "next", 8, &buffer))
        .await
        .unwrap();

    let seen = model.recorded_messages();
    // Turn 1 had 2 generate calls; turn 2's first call is the last recorded.
    let turn2_messages = seen.last().unwrap();
    let has_tool_call = turn2_messages.iter().any(|m| !m.tool_calls.is_empty());
    let has_tool_result = turn2_messages.iter().any(|m| m.tool_call_id.is_some());
    assert!(
        has_tool_call,
        "turn 2 should see turn 1's tool call in the buffer"
    );
    assert!(
        has_tool_result,
        "turn 2 should see turn 1's tool result in the buffer"
    );
}

#[tokio::test]
async fn turn_report_counts_steps_and_blocks() {
    // The per-turn observability span records `steps` (model calls) and `blocks` (run_lua
    // executions); the counts live on `TurnReport`, so this verifies the substance the span carries
    // without depending on `tracing`'s global subscriber state (spec §Observability → per-turn spans).

    // A single reply: one model step, no blocks.
    let h = Harness::new();
    let model = ScriptedModel::new([Completion::Reply("hi".to_owned())]);
    let report = run_turn(h.as_turn(&model, "hello", 8)).await.unwrap();
    assert_eq!(report.outcome, TurnOutcome::Reply("hi".to_owned()));
    assert_eq!(report.steps, 1);
    assert_eq!(report.blocks, 0);

    // A multi-step turn: two run_lua calls then a reply → three steps, two blocks.
    let h = Harness::new();
    let model = ScriptedModel::new([
        run_lua_call("return 1"),
        run_lua_call("return 2"),
        Completion::Reply("done".to_owned()),
    ]);
    let report = run_turn(h.as_turn(&model, "go", 8)).await.unwrap();
    assert_eq!(report.outcome, TurnOutcome::Reply("done".to_owned()));
    assert_eq!(report.steps, 3);
    assert_eq!(report.blocks, 2);
}

/// End-to-end against the real model (model-gated, ignored): the live model drives the whole loop
/// — chat protocol, tool-call threading, block execution — to a terminal without an infra error.
#[tokio::test]
#[ignore = "requires a reachable model endpoint (config.toml)"]
async fn real_model_drives_a_turn() {
    let Ok(config) = EnvConfig::load(std::path::Path::new("config.toml")) else {
        return;
    };
    if config.model.endpoint.is_empty() {
        eprintln!("skipping: no model endpoint configured");
        return;
    }
    let client = OpenAiClient::new(&config.model);
    let h = Harness::new();

    let outcome = run_turn(h.as_turn(
        &client,
        "Please remember that Dave climbs at the bouldering gym, then confirm you've noted it.",
        8,
    ))
    .await;

    match outcome {
        Ok(outcome) => {
            // The loop completed against the real model. Exactly one agent turn was recorded.
            assert_eq!(count_agent_turns(&h.events()), 1);
            eprintln!("real-model turn outcome: {outcome:?}");
        }
        Err(error) => eprintln!("skipping: {error}"),
    }
}

/// Temporal extraction against the real model (model-gated, ignored, tracked/non-gating): a turn
/// whose content carries natural-language times should leave at least one durable entry with a
/// resolved `occurred_at`. Logs the timed/total rate — load-bearing news about the model floor, the
/// same epistemic status as the compaction continuity metric (spec §Validation).
#[tokio::test]
#[ignore = "requires a reachable model endpoint (config.toml)"]
async fn real_model_extracts_temporal_references() {
    let Ok(config) = EnvConfig::load(std::path::Path::new("config.toml")) else {
        return;
    };
    if config.model.endpoint.is_empty() {
        eprintln!("skipping: no model endpoint configured");
        return;
    }
    let client = OpenAiClient::new(&config.model);
    let h = Harness::new();
    genesis::rollout(
        h.engine.store.lock().as_mut(),
        &h.clock,
        &seed(),
        None,
        &InstanceFeatures::default(),
    )
    .unwrap();
    h.engine
        .graph
        .lock()
        .materialize_from(h.engine.store.lock().as_ref())
        .unwrap();

    let outcome = run_turn(h.as_turn(
        &client,
        "Please note: I met Dave at the climbing gym last Tuesday, and the database migration \
         ships next Friday.",
        8,
    ))
    .await;
    if let Err(error) = outcome {
        eprintln!("skipping: {error}");
        return;
    }

    // Scan the namespaces a turn like this could write into for entries that gained an occurrence.
    let (mut total, mut timed) = (0usize, 0usize);
    for prefix in ["person/", "topic/", "project/", "event/"] {
        for memory in h.engine.graph.lock().memories_in_namespace(prefix).unwrap() {
            for entry in h.engine.graph.lock().entries_local(memory.id).unwrap() {
                total += 1;
                if entry.occurred_sort.is_some() {
                    timed += 1;
                    eprintln!("timed: {} :: {}", memory.name.as_str(), entry.text);
                }
            }
        }
    }
    eprintln!("temporal extraction: {timed}/{total} durable entries carry an occurred_at");
    assert!(
        timed >= 1,
        "expected the model to resolve at least one temporal reference"
    );
}

/// The `ModelCalled` events of a run, in `seq` order, projected to the fields the tests assert over.
fn model_calls(
    events: &[Event],
) -> Vec<(ModelPhase, Option<RequestRecord>, Option<String>, String)> {
    events
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::ModelCalled {
                phase,
                request,
                reasoning,
                request_digest,
                ..
            } => Some((
                *phase,
                request.clone(),
                reasoning.clone(),
                request_digest.clone(),
            )),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_turn_records_the_model_interaction_with_deliberation() {
    let h = Harness::new();
    let usage = Usage {
        prompt_tokens: Some(100),
        completion_tokens: Some(20),
        total_tokens: Some(120),
    };
    // Two steps so the delta path runs: a tool call, then a reply, each carrying reasoning.
    let model = ScriptedModel::with_deliberation([
        (
            run_lua_call(r#"memory.create(PERSON_DAVE, "Met at the gym")"#),
            "I should record Dave.".to_owned(),
            usage,
        ),
        (
            Completion::Reply("Noted.".to_owned()),
            "The fact is stored, so I reply.".to_owned(),
            usage,
        ),
    ]);

    run_turn(h.as_turn(&model, "Remember Dave", 8))
        .await
        .unwrap();

    let calls = model_calls(&h.events());
    // No description-regen template is registered (no genesis), so synthesis never runs: exactly the
    // two step calls are recorded.
    assert_eq!(calls.len(), 2, "one ModelCalled per step");
    assert!(calls.iter().all(|(phase, ..)| *phase == ModelPhase::Step));

    // The deliberation is captured verbatim.
    assert_eq!(calls[0].2.as_deref(), Some("I should record Dave."));
    assert_eq!(
        calls[1].2.as_deref(),
        Some("The fact is stored, so I reply.")
    );
    // Digests are present and differ — the second call's prompt grew.
    assert!(!calls[0].3.is_empty() && !calls[1].3.is_empty());
    assert_ne!(calls[0].3, calls[1].3);

    // The first call is a Base; the second a Continuation of the messages appended since.
    let RequestRecord::Base { messages: base, .. } =
        calls[0].1.clone().expect("Full captures request")
    else {
        panic!("the first step records a Base, got {:?}", calls[0].1);
    };
    let RequestRecord::Continuation { appended_messages } =
        calls[1].1.clone().expect("Full captures request")
    else {
        panic!("a later step records a Continuation, got {:?}", calls[1].1);
    };

    // Reconstructing the second call's prompt (Base ++ Continuation) reproduces exactly what the
    // model was sent on its second call.
    let reconstructed: Vec<Message> = base.iter().chain(&appended_messages).cloned().collect();
    let seen = model.recorded_messages();
    assert_eq!(seen.len(), 2);
    assert_eq!(reconstructed, seen[1]);

    // Block timing rides on the LuaExecuted (the field is recorded; it cannot be negative).
    let events = h.events();
    assert!(events.iter().any(|e| matches!(
        &e.payload,
        EventPayload::LuaExecuted { duration_ms, .. } if *duration_ms < u64::MAX
    )));
}

#[tokio::test]
async fn digest_capture_keeps_the_digest_but_drops_the_request() {
    let h = Harness::new();
    let model = ScriptedModel::new([Completion::Reply("Hi.".to_owned())]);

    run_turn(h.as_turn_capturing(&model, "Hello", 8, CaptureLevel::Digest))
        .await
        .unwrap();

    let calls = model_calls(&h.events());
    assert_eq!(calls.len(), 1);
    // The request is dropped, but the digest survives for an integrity check.
    assert!(calls[0].1.is_none(), "Digest drops the request payload");
    assert!(!calls[0].3.is_empty(), "Digest keeps the request digest");
}

#[tokio::test]
async fn off_capture_records_no_model_interaction() {
    let h = Harness::new();
    let model = ScriptedModel::new([Completion::Reply("Hi.".to_owned())]);

    run_turn(h.as_turn_capturing(&model, "Hello", 8, CaptureLevel::Off))
        .await
        .unwrap();

    assert!(
        model_calls(&h.events()).is_empty(),
        "Off emits no ModelCalled events"
    );
}

/// Supersession against the real model (model-gated, ignored). A realistic two-turn flow: the model
/// records a fact, then is told a correction in the same conversation, so it acts on the memory it
/// created rather than guessing a name. Observational, like the temporal-extraction probe — it prints
/// what the model did (did it supersede, append, or fragment into a duplicate?) and asserts only the
/// robust invariants: the turns complete without an infrastructure error, and our mechanism holds (an
/// entry recorded as superseded is never live). Whether the model *chooses* to supersede is a
/// model-floor observation, not a gate.
#[tokio::test]
#[ignore = "requires a reachable model endpoint (config.toml)"]
async fn real_model_supersedes_a_corrected_fact() {
    let Ok(config) = EnvConfig::load(std::path::Path::new("config.toml")) else {
        return;
    };
    if config.model.endpoint.is_empty() {
        eprintln!("skipping: no model endpoint configured");
        return;
    }
    let client = OpenAiClient::new(&config.model);
    let h = Harness::new();
    genesis::rollout(
        h.engine.store.lock().as_mut(),
        &h.clock,
        &seed(),
        None,
        &InstanceFeatures::default(),
    )
    .unwrap();
    h.engine
        .graph
        .lock()
        .materialize_from(h.engine.store.lock().as_ref())
        .unwrap();
    let conversation = h.session.conversation();

    // Turn 1: the model records the fact under a name of its own choosing.
    let first = run_turn(h.as_turn(&client, "Please remember that Dave works at Hooli.", 12)).await;
    let Ok(first) = first else {
        eprintln!("skipping: turn 1 failed: {first:?}");
        return;
    };
    eprintln!("turn 1 reply: {:?}", first.outcome);

    // Turn 2: a natural mention of the change — no instruction to update the record — with turn 1
    // replayed as the conversation buffer. The agent should infer that its memory of Dave is now stale
    // and maintain it on its own.
    let buffer = buffer_turns(h.engine.store.lock().as_ref(), conversation, Seq::ZERO).unwrap();
    let second = run_turn(h.as_turn_buffered(
        &client,
        "Oh, by the way — Dave left Hooli. He's over at Pied Piper these days.",
        12,
        &buffer,
    ))
    .await;
    let Ok(second) = second else {
        eprintln!("skipping: turn 2 failed: {second:?}");
        return;
    };
    eprintln!("turn 2 reply: {:?}", second.outcome);

    // What the model actually did, step by step — the deliberation surface.
    let superseded_ids: std::collections::BTreeSet<EntryId> = h
        .events()
        .into_iter()
        .filter_map(|event| match event.payload {
            EventPayload::LuaExecuted {
                script,
                result,
                terminal_cause,
                ..
            } => {
                eprintln!("  lua: {script:?} -> result={result:?} terminal={terminal_cause:?}");
                None
            }
            EventPayload::MemorySuperseded { entry, .. } => Some(entry),
            _ => None,
        })
        .collect();

    // The current person/* memories and their live/superseded entries.
    let people = h
        .engine
        .graph
        .lock()
        .memories_in_namespace("person/")
        .unwrap();
    eprintln!("person/* memories ({}):", people.len());
    let mut pied_piper_live = false;
    let mut hooli_live = false;
    for person in &people {
        let history = h.engine.graph.lock().class_history(person.id).unwrap();
        eprintln!("  {} ({} entries):", person.name.as_str(), history.len());
        for entry in &history {
            let live = entry.superseded_by.is_none();
            let lower = entry.text.to_lowercase();
            if live && lower.contains("pied piper") {
                pied_piper_live = true;
            }
            if live && lower.contains("hooli") {
                hooli_live = true;
            }
            eprintln!(
                "    - [{}] {}",
                if live { "live" } else { "superseded" },
                entry.text
            );
            // Mechanism invariant: a superseded entry must not appear in the live class read.
            if !live {
                let in_live = h
                    .engine
                    .graph
                    .lock()
                    .class_entries(person.id)
                    .unwrap()
                    .iter()
                    .any(|e| e.entry_id == entry.entry_id);
                assert!(!in_live, "a superseded entry is still in the live read");
            }
        }
    }

    eprintln!(
        "verdict: supersessions={}, person/* memories={}, 'Pied Piper' live={pied_piper_live}, \
         'Hooli' live={hooli_live}",
        superseded_ids.len(),
        people.len(),
    );
    if people.len() > 1 {
        eprintln!("  NOTE: the model fragmented Dave into multiple memories (name mismatch).");
    }
    if hooli_live && pied_piper_live {
        eprintln!(
            "  NOTE: both the stale and corrected facts are live — the model appended without retracting."
        );
    }
}

/// A `Completion::Reply` carrying a serialized [`LinkInferenceArgs`] reply.
fn link_inference_call(args: LinkInferenceArgs) -> Completion {
    Completion::Reply(serde_json::to_string(&args).unwrap())
}

#[tokio::test]
async fn link_inference_registers_and_links_from_content() {
    let h = Harness::new();
    genesis::rollout(
        h.engine.store.lock().as_mut(),
        &h.clock,
        &seed(),
        None,
        &InstanceFeatures::default(),
    )
    .unwrap();
    h.engine
        .graph
        .lock()
        .materialize_from(h.engine.store.lock().as_ref())
        .unwrap();
    h.baseline_descriptions();
    h.baseline_link_inference();

    // The turn creates person/dave and topic/zephyr, appending a public entry about authorship.
    // The agent does NOT call mem:link — the inference pass should extract the relationship.
    let model = ScriptedModel::new([
        run_lua_call(
            r#"memory.create(PERSON_DAVE, "a person")
               local zephyr = memory.create("topic/zephyr")
               zephyr:append("Authored by Dave", { by_agent = true, visibility = "public" })"#,
        ),
        Completion::Reply("Noted.".to_owned()),
        // The describe catch-up synthesizes both written memories: dave first ("a person"), then
        // zephyr (the authorship entry). Each gets its own synthesis call.
        synthesize_call(SynthesizeReply::description("A person.")),
        synthesize_call(SynthesizeReply::description("The zephyr project.")),
        // The link-inference call: register authored_by and link zephyr → dave.
        link_inference_call(LinkInferenceArgs {
            new_relations: vec![NewRelationSpec {
                name: "authored_by".to_owned(),
                inverse: "authored".to_owned(),
                from_card: "many".to_owned(),
                to_card: "one".to_owned(),
                symmetric: false,
                reflexive: false,
                description: String::new(),
            }],
            links: vec![InferredLink {
                entry: 1,
                relation: "authored_by".to_owned(),
                target: "person/dave".to_owned(),
                direction: "to".to_owned(),
            }],
        }),
    ]);

    run_turn(h.as_turn(&model, "Working on zephyr, authored by Dave", 8))
        .await
        .unwrap();
    h.describe(&model).await;
    h.link_inference(&model).await;

    let events = h.events();

    // A LinkTypeRegistered for authored_by was committed.
    let registered = events.iter().any(|e| {
        matches!(
            &e.payload,
            EventPayload::LinkTypeRegistered { name, .. } if name.as_str() == "authored_by"
        )
    });
    assert!(
        registered,
        "the pass should register the authored_by relation"
    );

    // An inferred LinkCreated from topic/zephyr to person/dave under authored_by.
    let dave = h
        .engine
        .graph
        .lock()
        .memory_by_name(Namespace::Person.with_name("dave"))
        .unwrap()
        .unwrap();
    let zephyr = h
        .engine
        .graph
        .lock()
        .memory_by_name(Namespace::Topic.with_name("zephyr"))
        .unwrap()
        .unwrap();

    let linked = events.iter().any(|e| {
        matches!(
            &e.payload,
            EventPayload::LinkCreated { from, to, relation, source, .. }
            if *from == zephyr.id && *to == dave.id
              && relation.as_str() == "authored_by"
              && *source == zuihitsu::LinkSource::Inferred
        )
    });
    assert!(
        linked,
        "the pass should create an inferred authored_by link"
    );
}

#[tokio::test]
async fn link_inference_honors_a_seeded_inverse_label() {
    // The model is shown both labels of every registered pair, so it legitimately phrases a link
    // through the inverse — `created` for the seeded `created_by` — with no new registration to
    // propose. The pass must resolve it onto the canonical relation with the direction flipped,
    // not drop it as unregistered.
    let h = Harness::new();
    genesis::rollout(
        h.engine.store.lock().as_mut(),
        &h.clock,
        &seed(),
        None,
        &InstanceFeatures::default(),
    )
    .unwrap();
    h.engine
        .graph
        .lock()
        .materialize_from(h.engine.store.lock().as_ref())
        .unwrap();
    h.baseline_descriptions();
    h.baseline_link_inference();

    let model = ScriptedModel::new([
        run_lua_call(
            r#"memory.create("person/clara", "a person")
               local zephyr = memory.create("topic/zephyr")
               zephyr:append("This project was created by Clara", { by_agent = true, visibility = "public" })"#,
        ),
        Completion::Reply("Noted.".to_owned()),
        synthesize_call(SynthesizeReply::description("A person.")),
        synthesize_call(SynthesizeReply::description("The zephyr project.")),
        // The inference reply names the edge through the seeded inverse, registering nothing new:
        // clara --created--> zephyr, the same fact as zephyr --created_by--> clara.
        link_inference_call(LinkInferenceArgs {
            new_relations: vec![],
            links: vec![InferredLink {
                entry: 1,
                relation: "created".to_owned(),
                target: "person/clara".to_owned(),
                direction: "from".to_owned(),
            }],
        }),
    ]);

    run_turn(h.as_turn(&model, "This project was created by Clara", 8))
        .await
        .unwrap();
    h.describe(&model).await;
    h.link_inference(&model).await;

    let clara = h
        .engine
        .graph
        .lock()
        .memory_by_name(Namespace::Person.with_name("clara"))
        .unwrap()
        .unwrap();
    let zephyr = h
        .engine
        .graph
        .lock()
        .memory_by_name(Namespace::Topic.with_name("zephyr"))
        .unwrap()
        .unwrap();

    let linked = h.events().iter().any(|e| {
        matches!(
            &e.payload,
            EventPayload::LinkCreated { from, to, relation, source, .. }
            if *from == zephyr.id && *to == clara.id
              && relation.as_str() == "created_by"
              && *source == zuihitsu::LinkSource::Inferred
        )
    });
    assert!(
        linked,
        "an inverse-labeled inferred link should land on the canonical relation, direction flipped"
    );
}

#[tokio::test]
async fn link_inference_is_idempotent() {
    let h = Harness::new();
    genesis::rollout(
        h.engine.store.lock().as_mut(),
        &h.clock,
        &seed(),
        None,
        &InstanceFeatures::default(),
    )
    .unwrap();
    h.engine
        .graph
        .lock()
        .materialize_from(h.engine.store.lock().as_ref())
        .unwrap();
    h.baseline_descriptions();

    h.baseline_link_inference();
    let model = ScriptedModel::new([
        run_lua_call(
            r#"memory.create(PERSON_DAVE, "a person")
               local zephyr = memory.create("topic/zephyr")
               zephyr:append("Authored by Dave", { by_agent = true, visibility = "public" })"#,
        ),
        Completion::Reply("Noted.".to_owned()),
        synthesize_call(SynthesizeReply::description("A person.")),
        synthesize_call(SynthesizeReply::description("The zephyr project.")),
        link_inference_call(LinkInferenceArgs {
            new_relations: vec![NewRelationSpec {
                name: "authored_by".to_owned(),
                inverse: "authored".to_owned(),
                from_card: "many".to_owned(),
                to_card: "one".to_owned(),
                symmetric: false,
                reflexive: false,
                description: String::new(),
            }],
            links: vec![InferredLink {
                entry: 1,
                relation: "authored_by".to_owned(),
                target: "person/dave".to_owned(),
                direction: "to".to_owned(),
            }],
        }),
    ]);

    run_turn(h.as_turn(&model, "Working on zephyr, authored by Dave", 8))
        .await
        .unwrap();
    h.describe(&model).await;
    h.link_inference(&model).await;

    // Count inferred LinkCreated events after the first pass.
    let first_count = h
        .events()
        .into_iter()
        .filter(|e| {
            matches!(
                &e.payload,
                EventPayload::LinkCreated {
                    source: zuihitsu::LinkSource::Inferred,
                    ..
                }
            )
        })
        .count();

    // Re-run from the same cursor — no new events (the cursor advance prevents re-scanning).
    h.link_inference(&model).await;
    let second_count = h
        .events()
        .into_iter()
        .filter(|e| {
            matches!(
                &e.payload,
                EventPayload::LinkCreated {
                    source: zuihitsu::LinkSource::Inferred,
                    ..
                }
            )
        })
        .count();
    assert_eq!(
        first_count, second_count,
        "re-running should produce no new inferred links"
    );
}

#[tokio::test]
async fn link_inference_degrades_gracefully_with_no_usable_reply() {
    let h = Harness::new();
    genesis::rollout(
        h.engine.store.lock().as_mut(),
        &h.clock,
        &seed(),
        None,
        &InstanceFeatures::default(),
    )
    .unwrap();
    h.engine
        .graph
        .lock()
        .materialize_from(h.engine.store.lock().as_ref())
        .unwrap();
    h.baseline_descriptions();
    h.baseline_link_inference();

    let model = ScriptedModel::new([
        run_lua_call(
            r#"memory.create(PERSON_DAVE, "a person")
               local zephyr = memory.create("topic/zephyr")
               zephyr:append("Authored by Dave", { by_agent = true, visibility = "public" })"#,
        ),
        Completion::Reply("Noted.".to_owned()),
        synthesize_call(SynthesizeReply::description("A person.")),
        synthesize_call(SynthesizeReply::description("The zephyr project.")),
    ]);

    run_turn(h.as_turn(&model, "Working on zephyr, authored by Dave", 8))
        .await
        .unwrap();
    h.describe(&model).await;

    // Run the pass with a model that has no more scripted completions — each generate call
    // returns Exhausted, which the pass logs and skips (graceful degradation: a failed inference
    // leaves the memory unchanged — no link, no harm).
    h.link_inference(&ScriptedModel::new([])).await;

    let inferred = h
        .events()
        .into_iter()
        .filter(|e| {
            matches!(
                &e.payload,
                EventPayload::LinkCreated {
                    source: zuihitsu::LinkSource::Inferred,
                    ..
                }
            )
        })
        .count();
    assert_eq!(
        inferred, 0,
        "no inferred links should be created with no usable reply"
    );
}

/// With `linking` disabled, calling `:link` from Lua yields a nil-call error (the method is not
/// installed), while the link-inference pass — which runs as a model call, not through the agent's
/// Lua — still creates the link. This is the AC.9 contract: the three gates (Lua registration, API
/// reference, scaffold) all drop linking, so the inference pass is the sole path to a link.
#[tokio::test]
async fn disabled_linking_rejects_mem_link_but_inference_still_links() {
    let disabled = InstanceFeatures {
        linking: false,
        ..Default::default()
    };
    let h = Harness::with_features(disabled);
    genesis::rollout(
        h.engine.store.lock().as_mut(),
        &h.clock,
        &seed(),
        None,
        &disabled,
    )
    .unwrap();
    h.engine
        .graph
        .lock()
        .materialize_from(h.engine.store.lock().as_ref())
        .unwrap();
    h.baseline_descriptions();
    h.baseline_link_inference();

    // Create the two memories through a turn (the agent-authored path that sets visibility
    // correctly), appending the authorship entry to topic/zephyr — but NOT calling mem:link.
    let create_model = ScriptedModel::new([
        run_lua_call(
            r#"memory.create(PERSON_DAVE, "a person")
               local zephyr = memory.create("topic/zephyr")
               zephyr:append("Authored by Dave", { by_agent = true, visibility = "public" })"#,
        ),
        Completion::Reply("Noted.".to_owned()),
    ]);
    run_turn(h.as_turn(&create_model, "Working on zephyr, authored by Dave", 8))
        .await
        .unwrap();

    // Now attempt a `:link` call directly — it must fail because `:link` is not installed when
    // linking is disabled. The block terminates with Luau's absent-method call error, which names
    // the missing method ("attempt to call missing method 'link' of table").
    let link_outcome = h
        .run(r#"memory.get("topic/zephyr"):link("authored_by", memory.get(PERSON_DAVE))"#)
        .await;
    match link_outcome {
        BlockOutcome::Terminated(TerminalCause::Error(message)) => {
            assert!(
                message.contains("'link'") && message.contains("missing method"),
                "a disabled :link should surface an absent-method call error, got: {message}"
            );
        }
        other => panic!("expected the :link call to terminate, got {other:?}"),
    }

    // The link was not created by the agent (the block terminated before it). Now drive the
    // link-inference pass, which runs as a model call — it should still create the authored_by link.
    let inference_model = ScriptedModel::new([
        synthesize_call(SynthesizeReply::description("A person.")),
        synthesize_call(SynthesizeReply::description("The zephyr project.")),
        link_inference_call(LinkInferenceArgs {
            new_relations: vec![NewRelationSpec {
                name: "authored_by".to_owned(),
                inverse: "authored".to_owned(),
                from_card: "many".to_owned(),
                to_card: "one".to_owned(),
                symmetric: false,
                reflexive: false,
                description: String::new(),
            }],
            links: vec![InferredLink {
                entry: 1,
                relation: "authored_by".to_owned(),
                target: "person/dave".to_owned(),
                direction: "to".to_owned(),
            }],
        }),
    ]);
    h.describe(&inference_model).await;
    h.link_inference(&inference_model).await;

    let events = h.events();
    let linked = events.iter().any(|e| {
        matches!(
            &e.payload,
            EventPayload::LinkCreated { relation, source, .. }
            if relation.as_str() == "authored_by"
              && *source == zuihitsu::LinkSource::Inferred
        )
    });
    assert!(
        linked,
        "the inference pass should still create the authored_by link when linking is disabled"
    );
}
