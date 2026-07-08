use super::{
    EntryId, EventPayload, Initiation, MemoryId, MergeProposalSource, ModelPhase, RequestRecord,
    Teller, TurnRole, Visibility,
};
use crate::{
    brief::{Brief, BriefFact, BriefRelationship},
    ids::{ConversationId, MemoryName, SessionId, TurnId},
    model::{Completion, Message, ToolChoice, Usage},
    time::{CivilDate, TemporalRef, Timestamp},
    vocabulary::RelationName,
};

fn join_turn(brief: Option<Brief>) -> EventPayload {
    EventPayload::ConversationTurn {
        conversation: ConversationId::generate(),
        turn_id: TurnId::generate(),
        role: TurnRole::System,
        text: "## person/priya\n".to_owned(),
        participant: Some(MemoryId::generate()),
        initiation: Initiation::Responding,
        produced_by: None,
        brief,
    }
}

fn representative_brief() -> Brief {
    Brief {
        subject: MemoryName::new("person/priya"),
        summary: Some("Priya, staff engineer".to_owned()),
        recent_facts: vec![BriefFact {
            text: "weighing an offer".to_owned(),
            markers: vec!["[via person/erin]".to_owned()],
        }],
        relationships: vec![BriefRelationship {
            relation: RelationName::new("knows"),
            subject: MemoryName::new("person/erin"),
            marker: None,
        }],
    }
}

#[test]
fn conversation_turn_round_trips_a_structured_brief() {
    let event = join_turn(Some(representative_brief()));
    let json = serde_json::to_string(&event).unwrap();
    assert_eq!(serde_json::from_str::<EventPayload>(&json).unwrap(), event);
}

#[test]
fn conversation_turn_without_a_brief_replays_as_none() {
    // A version-1 `ConversationTurn` predates the `brief` field; dropping the key models an old
    // log. `serde(default)` must fill `None` so the historical turn deserializes unchanged.
    let mut value = serde_json::to_value(join_turn(Some(representative_brief()))).unwrap();
    value.as_object_mut().unwrap().remove("brief");
    let replayed: EventPayload = serde_json::from_value(value).unwrap();
    assert!(matches!(
        replayed,
        EventPayload::ConversationTurn { brief: None, .. }
    ));
}

fn content_with(occurred_at: Option<TemporalRef>) -> EventPayload {
    EventPayload::MemoryContentAppended {
        id: MemoryId::generate(),
        entry_id: EntryId::generate(),
        asserted_at: Timestamp::from_millis(1),
        occurred_at,
        text: "x".to_owned(),
        told_by: Teller::Agent,
        told_in: None,
        visibility: Visibility::Public,
    }
}

#[test]
fn content_append_without_occurred_at_replays_as_none() {
    // A pre-Stage-9 payload predates the field; dropping the key models an old log. `serde(default)`
    // must fill `None` so the historical event deserializes unchanged.
    let mut value = serde_json::to_value(content_with(Some(TemporalRef::Day(CivilDate(
        "2026-06-03".into(),
    )))))
    .unwrap();
    value.as_object_mut().unwrap().remove("occurred_at");
    let replayed: EventPayload = serde_json::from_value(value).unwrap();
    assert!(matches!(
        replayed,
        EventPayload::MemoryContentAppended {
            occurred_at: None,
            ..
        }
    ));
}

#[test]
fn content_append_round_trips_occurred_at() {
    let event = content_with(Some(TemporalRef::Instant(Timestamp::from_millis(42))));
    let json = serde_json::to_string(&event).unwrap();
    assert_eq!(serde_json::from_str::<EventPayload>(&json).unwrap(), event);
}

#[test]
fn entry_temporal_resolved_round_trips() {
    let event = EventPayload::EntryTemporalResolved {
        id: MemoryId::generate(),
        entry_id: EntryId::generate(),
        occurred_at: TemporalRef::Day(CivilDate("2026-06-03".into())),
        produced_by: None,
    };
    let json = serde_json::to_string(&event).unwrap();
    assert_eq!(serde_json::from_str::<EventPayload>(&json).unwrap(), event);
}

#[test]
fn entry_temporal_resolve_failed_round_trips() {
    let event = EventPayload::EntryTemporalResolveFailed {
        id: MemoryId::generate(),
        entry_id: EntryId::generate(),
        raw: "{\"recurring\":\"every Monday\"}".to_owned(),
        reason: "unsupported recurrence rule".to_owned(),
        produced_by: None,
    };
    let json = serde_json::to_string(&event).unwrap();
    assert_eq!(serde_json::from_str::<EventPayload>(&json).unwrap(), event);
}

#[test]
fn memory_superseded_round_trips() {
    let event = EventPayload::MemorySuperseded {
        id: MemoryId::generate(),
        entry: EntryId::generate(),
        superseded_by: EntryId::generate(),
    };
    let json = serde_json::to_string(&event).unwrap();
    assert_eq!(serde_json::from_str::<EventPayload>(&json).unwrap(), event);
}

#[test]
fn scheduled_job_fired_round_trips() {
    let event = EventPayload::ScheduledJobFired {
        entry_id: EntryId::generate(),
        memory: MemoryId::generate(),
        fired_at: Timestamp::from_millis(1_000),
    };
    let json = serde_json::to_string(&event).unwrap();
    assert_eq!(serde_json::from_str::<EventPayload>(&json).unwrap(), event);
}

#[test]
fn belief_arbitrated_round_trips() {
    let event = EventPayload::BeliefArbitrated {
        memory: MemoryId::generate(),
        competing_entries: vec![EntryId::generate(), EntryId::generate()],
        resolution: super::ArbitrationResolution {
            credited: vec![EntryId::generate()],
            statement: "credited the more recent assertion".to_owned(),
        },
        produced_by: None,
    };
    let json = serde_json::to_string(&event).unwrap();
    assert_eq!(serde_json::from_str::<EventPayload>(&json).unwrap(), event);
}

#[test]
fn model_called_round_trips() {
    let event = EventPayload::ModelCalled {
        conversation: ConversationId::generate(),
        turn_id: TurnId::generate(),
        phase: ModelPhase::Step,
        request_digest: "abc123".to_owned(),
        request: Some(RequestRecord::Base {
            system: "be concise".to_owned(),
            messages: vec![Message::user("hi")],
            tools: Vec::new(),
            tool_choice: ToolChoice::Auto,
            thinking: None,
        }),
        completion: Completion::Reply("hello".to_owned()),
        reasoning: Some("they greeted me".to_owned()),
        finish_reason: Some("stop".to_owned()),
        usage: Usage {
            prompt_tokens: Some(10),
            completion_tokens: Some(2),
            total_tokens: Some(12),
        },
        duration_ms: 1_234,
    };
    let json = serde_json::to_string(&event).unwrap();
    assert_eq!(serde_json::from_str::<EventPayload>(&json).unwrap(), event);
}

#[test]
fn lua_executed_without_duration_replays_as_zero() {
    // A pre-timing `LuaExecuted` predates `duration_ms`; dropping the key models an old log, and
    // `serde(default)` must fill `0` so the historical event still deserializes.
    let event = EventPayload::LuaExecuted {
        conversation: ConversationId::generate(),
        turn_id: TurnId::generate(),
        script: "return 1".to_owned(),
        result: Some("1".to_owned()),
        touched: Vec::new(),
        terminal_cause: None,
        duration_ms: 99,
    };
    let mut value = serde_json::to_value(&event).unwrap();
    value.as_object_mut().unwrap().remove("duration_ms");
    assert!(matches!(
        serde_json::from_value::<EventPayload>(value).unwrap(),
        EventPayload::LuaExecuted { duration_ms: 0, .. }
    ));
}

#[test]
fn scheduled_item_surfaced_round_trips() {
    let event = EventPayload::ScheduledItemSurfaced {
        entry_id: EntryId::generate(),
        memory: MemoryId::generate(),
        session: SessionId::generate(),
        surfaced_at: Timestamp::from_millis(2_000),
    };
    let json = serde_json::to_string(&event).unwrap();
    assert_eq!(serde_json::from_str::<EventPayload>(&json).unwrap(), event);
}

#[test]
fn describe_pass_completed_round_trips() {
    let event =
        EventPayload::describe_pass_completed(vec![MemoryId::generate(), MemoryId::generate()]);
    let json = serde_json::to_string(&event).unwrap();
    assert_eq!(serde_json::from_str::<EventPayload>(&json).unwrap(), event);
}

#[test]
fn a_version_one_merge_proposed_reads_as_agent_sourced() {
    // A payload written before `source` and `rationale` existed must replay: both fields default
    // (to `Agent` and no stated grounds), which every early proposal was.
    let from = MemoryId::generate();
    let to = MemoryId::generate();
    let legacy = format!(
        r#"{{"type":"MergeProposed","from":"{}","to":"{}"}}"#,
        from.0, to.0
    );
    assert_eq!(
        serde_json::from_str::<EventPayload>(&legacy).unwrap(),
        EventPayload::MergeProposed {
            from,
            to,
            source: MergeProposalSource::Agent,
            rationale: None,
        }
    );
}

#[test]
fn a_version_two_merge_proposed_reads_with_no_rationale() {
    // A payload written after `source` but before `rationale` replays with the rationale absent.
    let from = MemoryId::generate();
    let to = MemoryId::generate();
    let legacy = format!(
        r#"{{"type":"MergeProposed","from":"{}","to":"{}","source":"Orchestration"}}"#,
        from.0, to.0
    );
    assert_eq!(
        serde_json::from_str::<EventPayload>(&legacy).unwrap(),
        EventPayload::MergeProposed {
            from,
            to,
            source: MergeProposalSource::Orchestration,
            rationale: None,
        }
    );
}

#[test]
fn cardinality_from_str_matches_case_insensitively() {
    use super::Cardinality;
    assert_eq!("one".parse::<Cardinality>(), Ok(Cardinality::One));
    assert_eq!("One".parse::<Cardinality>(), Ok(Cardinality::One));
    assert_eq!("many".parse::<Cardinality>(), Ok(Cardinality::Many));
    assert_eq!("MANY".parse::<Cardinality>(), Ok(Cardinality::Many));
    assert!("several".parse::<Cardinality>().is_err());
    assert!("".parse::<Cardinality>().is_err());
}
