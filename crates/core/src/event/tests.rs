use crate::{
    brief::{Brief, BriefFact, BriefRelationship},
    event::{
        EntryId, Event, EventPayload, EventSource, Initiation, LinkSource, MemoryId,
        MergeProposalSource, ModelPhase, RequestRecord, Teller, TurnRole, Visibility,
    },
    ids::{ConversationId, MemoryName, Seq, SessionId, TurnId},
    model::{Completion, Message, ToolChoice, Usage},
    prompt::{PromptSectionKind, PromptSectionSpan},
    settings::Settings,
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
            source: MemoryName::new("person/priya"),
            target: MemoryName::new("person/erin"),
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

#[test]
fn a_join_turn_with_a_pre_pairing_brief_reconstructs_endpoints() {
    // A join-turn recorded before a brief relationship named both endpoints stored only the neighbour
    // as `subject`, with this identity the implicit near end and the edge rendered outgoing. Loading
    // such a log must reconstruct `source` (the brief's own subject) and `target` (the neighbour), so an
    // old join replays rather than failing to deserialize — the `#[serde(try_from = "BriefWire")]` path.
    let mut value = serde_json::to_value(join_turn(Some(representative_brief()))).unwrap();
    let relationship = value["brief"]["relationships"][0].as_object_mut().unwrap();
    relationship.remove("source");
    relationship.remove("target");
    relationship.insert("subject".to_owned(), serde_json::json!("person/erin"));

    let replayed: EventPayload = serde_json::from_value(value).unwrap();
    let EventPayload::ConversationTurn {
        brief: Some(brief), ..
    } = replayed
    else {
        panic!("the join turn carries a brief");
    };
    assert_eq!(
        brief.relationships,
        vec![BriefRelationship {
            relation: RelationName::new("knows"),
            source: MemoryName::new("person/priya"),
            target: MemoryName::new("person/erin"),
            marker: None,
        }]
    );
}

#[test]
fn class_primary_designation_without_the_flag_replays_as_a_pin() {
    // The earliest shape carried only `memory` and meant "designate". A payload missing `designated`
    // models such a log; the field's default must fill `true` so replay does not silently release the
    // operator's pin (the bool `Default` would give `false`).
    let mut value = serde_json::to_value(EventPayload::class_primary_designated(
        MemoryId::generate(),
        true,
    ))
    .unwrap();
    value.as_object_mut().unwrap().remove("designated");
    let replayed: EventPayload = serde_json::from_value(value).unwrap();
    assert!(matches!(
        replayed,
        EventPayload::ClassPrimaryDesignated {
            designated: true,
            ..
        }
    ));
}

#[test]
fn class_primary_designation_round_trips_a_release() {
    let event = EventPayload::class_primary_designated(MemoryId::generate(), false);
    let json = serde_json::to_string(&event).unwrap();
    assert_eq!(serde_json::from_str::<EventPayload>(&json).unwrap(), event);
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
    let id = MemoryId::generate();
    let entry_id = EntryId::generate();
    // Both a resolution (`Some`) and a withdrawal (`None`) survive the wire.
    let resolved = EventPayload::EntryTemporalResolved {
        id,
        entry_id,
        occurred_at: Some(TemporalRef::Day(CivilDate("2026-06-03".into()))),
        produced_by: None,
    };
    let withdrawn = EventPayload::EntryTemporalResolved {
        id,
        entry_id,
        occurred_at: None,
        produced_by: None,
    };
    for event in [&resolved, &withdrawn] {
        let json = serde_json::to_string(event).unwrap();
        assert_eq!(&serde_json::from_str::<EventPayload>(&json).unwrap(), event);
    }
    // `Some` serializes transparently — the temporal reference sits inline, exactly as a log written
    // before withdrawal existed carries it, with no `null` — so old logs deserialize as `Some` and
    // replay identically. A withdrawal is the only shape that writes `occurred_at: null`.
    let resolved_json = serde_json::to_string(&resolved).unwrap();
    assert!(resolved_json.contains("\"occurred_at\":{\"day\":"));
    assert!(
        serde_json::to_string(&withdrawn)
            .unwrap()
            .contains("\"occurred_at\":null")
    );
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
fn entries_consolidated_round_trips() {
    let event = EventPayload::entries_consolidated(
        MemoryId::generate(),
        vec![
            EntryId::generate(),
            EntryId::generate(),
            EntryId::generate(),
        ],
        EntryId::generate(),
        None,
    );
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
            system_sections: vec![PromptSectionSpan {
                kind: PromptSectionKind::Scaffold,
                start: 0,
                end: 11,
            }],
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
            cache_read_tokens: Some(8),
            cache_write_tokens: None,
        },
        duration_ms: 1_234,
    };
    let json = serde_json::to_string(&event).unwrap();
    assert_eq!(serde_json::from_str::<EventPayload>(&json).unwrap(), event);
}

#[test]
fn model_called_without_system_sections_replays_as_empty() {
    // A `ModelCalled` recorded before the prompt sections were captured has no `system_sections` key
    // on its `Base` request; `serde(default)` must fill an empty vec so the historical event still
    // deserializes and the console falls back to deriving the section boundaries itself.
    let event = EventPayload::ModelCalled {
        conversation: ConversationId::generate(),
        turn_id: TurnId::generate(),
        phase: ModelPhase::Step,
        request_digest: "abc123".to_owned(),
        request: Some(RequestRecord::Base {
            system: "be concise".to_owned(),
            // Immaterial: the `system_sections` key is stripped from the serialized JSON below, so the
            // constructed value never reaches the deserializer under test.
            system_sections: Vec::new(),
            messages: vec![Message::user("hi")],
            tools: Vec::new(),
            tool_choice: ToolChoice::Auto,
            thinking: None,
        }),
        completion: Completion::Reply("hello".to_owned()),
        reasoning: None,
        finish_reason: None,
        usage: Usage::default(),
        duration_ms: 0,
    };
    let mut value = serde_json::to_value(&event).unwrap();
    value["request"]["Base"]
        .as_object_mut()
        .unwrap()
        .remove("system_sections");

    let replayed = serde_json::from_value::<EventPayload>(value).unwrap();
    let EventPayload::ModelCalled {
        request: Some(RequestRecord::Base {
            system_sections, ..
        }),
        ..
    } = replayed
    else {
        panic!("expected a ModelCalled with a Base request, got {replayed:?}");
    };
    assert!(
        system_sections.is_empty(),
        "a pre-field Base defaults to no sections"
    );
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
        r#"{{"type":"MergeProposed","from":"{}","to":"{}","source":"Agent"}}"#,
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
fn a_usage_without_cache_fields_replays_as_unknown() {
    // A usage recorded before cache capture has no cache keys; `serde(default)` must fill `None`
    // (unknown, not zero) so the historical `ModelCalled` still deserializes, and a modern usage
    // round-trips its cache counts intact.
    let old = r#"{"prompt_tokens":10,"completion_tokens":2,"total_tokens":12}"#;
    let replayed: Usage = serde_json::from_str(old).unwrap();
    assert_eq!(replayed.cache_read_tokens, None);
    assert_eq!(replayed.cache_write_tokens, None);

    let modern = Usage {
        prompt_tokens: Some(10),
        cache_read_tokens: Some(8),
        ..Usage::default()
    };
    let json = serde_json::to_string(&modern).unwrap();
    assert_eq!(serde_json::from_str::<Usage>(&json).unwrap(), modern);
}

#[test]
fn a_session_started_without_a_working_set_replays_as_empty() {
    // A `SessionStarted` recorded before working-set capture has no `working_set` key; the field
    // defaults empty so the historical event still deserializes, and consumers distinguish
    // "recorded before capture" from "genuinely empty" by the key's presence in the raw payload.
    let event = EventPayload::SessionStarted {
        conversation: ConversationId::generate(),
        id: SessionId::generate(),
        participants: vec![MemoryId::generate()],
        started_at: Timestamp::from_millis(1_000),
        seeded_from_turn: None,
        brief: "the brief".to_owned(),
        working_set: Vec::new(),
        initiators: Vec::new(),
    };
    let mut value = serde_json::to_value(&event).unwrap();
    value.as_object_mut().unwrap().remove("working_set");

    let replayed = serde_json::from_value::<EventPayload>(value).unwrap();
    let EventPayload::SessionStarted { working_set, .. } = replayed else {
        panic!("expected a SessionStarted, got {replayed:?}");
    };
    assert!(working_set.is_empty());
}

#[test]
fn a_session_started_without_initiators_replays_as_empty() {
    // A `SessionStarted` recorded before initiator capture has no `initiators` key; the field defaults
    // empty so the historical event still deserializes.
    let event = EventPayload::SessionStarted {
        conversation: ConversationId::generate(),
        id: SessionId::generate(),
        participants: vec![MemoryId::generate()],
        started_at: Timestamp::from_millis(1_000),
        seeded_from_turn: None,
        brief: "the brief".to_owned(),
        working_set: Vec::new(),
        initiators: Vec::new(),
    };
    let mut value = serde_json::to_value(&event).unwrap();
    value.as_object_mut().unwrap().remove("initiators");

    let replayed = serde_json::from_value::<EventPayload>(value).unwrap();
    let EventPayload::SessionStarted { initiators, .. } = replayed else {
        panic!("expected a SessionStarted, got {replayed:?}");
    };
    assert!(initiators.is_empty());
}

fn stamped(source: EventSource) -> Event {
    Event {
        seq: Seq(7),
        recorded_at: Timestamp::from_millis(1_000),
        source,
        payload: EventPayload::memory_created(
            MemoryId::generate(),
            MemoryName::new("person/priya"),
        ),
    }
}

#[test]
fn an_event_round_trips_its_source() {
    // The `Event` envelope is a wire — it rides verbatim over the observability surfaces and into the
    // eval package — so its `source` must survive a serialize/deserialize round trip intact.
    for source in [
        EventSource::Bootstrap,
        EventSource::Agent,
        EventSource::Operator,
        EventSource::Orchestration,
    ] {
        let event = stamped(source);
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(serde_json::from_str::<Event>(&json).unwrap(), event);
    }
}

#[test]
fn a_pre_source_event_replays_as_agent() {
    // An envelope written before the `source` field existed has no `source` key; `serde(default)` must
    // fill `EventSource::Agent` — the historical fallback — so the old log still deserializes.
    let mut value = serde_json::to_value(stamped(EventSource::Operator)).unwrap();
    value.as_object_mut().unwrap().remove("source");
    let replayed: Event = serde_json::from_value(value).unwrap();
    assert_eq!(replayed.source, EventSource::Agent);
}

#[test]
fn event_source_debugger_alias_reads_as_operator() {
    // `Debugger` was this variant's serialized name before the operator interface was renamed; the
    // alias keeps an envelope written under the old name readable.
    let mut value = serde_json::to_value(stamped(EventSource::Operator)).unwrap();
    value["source"] = serde_json::json!("Debugger");
    let replayed: Event = serde_json::from_value(value).unwrap();
    assert_eq!(replayed.source, EventSource::Operator);
}

#[test]
fn a_retired_config_set_source_still_deserialises() {
    // The `source` field on `ConfigSet` and `PromptTemplateRegistered` is retired — the authoring
    // authority now rides on the envelope — but a log written before the retirement still carries the
    // key. Its `serde(default)` must let the old payload deserialize unchanged, and the current write
    // path must no longer emit the key.
    let legacy_config = r#"{"type":"ConfigSet","settings":{},"source":"Operator"}"#;
    assert!(matches!(
        serde_json::from_str::<EventPayload>(legacy_config).unwrap(),
        EventPayload::ConfigSet { .. }
    ));

    let legacy_template = r#"{"type":"PromptTemplateRegistered","name":"scaffold","version":1,"body":"x","source":"Orchestration"}"#;
    assert!(matches!(
        serde_json::from_str::<EventPayload>(legacy_template).unwrap(),
        EventPayload::PromptTemplateRegistered { .. }
    ));

    // The current serialization omits the retired key entirely.
    let config_json = serde_json::to_value(EventPayload::config_set(Settings::default())).unwrap();
    assert!(config_json.as_object().unwrap().get("source").is_none());
    let template_json = serde_json::to_value(EventPayload::prompt_template_registered(
        super::PromptTemplateName::Scaffold,
        1,
        "x",
    ))
    .unwrap();
    assert!(template_json.as_object().unwrap().get("source").is_none());
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

#[test]
fn model_call_aborted_round_trips() {
    // The abort record is a real wire: a discarded streaming attempt lands durably so the retry is
    // visible after the fact, and an old console (or a rejudge) must read it back whole.
    let event = EventPayload::ModelCallAborted {
        conversation: ConversationId::generate(),
        turn_id: TurnId::generate(),
        phase: ModelPhase::Step,
        attempt: 2,
        cause: "model: m: http error: connection reset".to_owned(),
        partial_reasoning: "Considering the".to_owned(),
        partial_reply: "Hello th".to_owned(),
    };
    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains("\"ModelCallAborted\""));
    let back: EventPayload = serde_json::from_str(&json).unwrap();
    match back {
        EventPayload::ModelCallAborted {
            attempt,
            cause,
            partial_reply,
            ..
        } => {
            assert_eq!(attempt, 2);
            assert!(cause.contains("connection reset"));
            assert_eq!(partial_reply, "Hello th");
        }
        other => panic!("expected the abort back, got {other:?}"),
    }
}

#[test]
fn link_source_platform_connector_round_trips_through_the_stored_label() {
    // The graph `links.source` column stores `LinkSource::as_str` and reads it back with `FromStr`, so
    // a connector edge must round-trip its identifier through that label — not just through serde.
    let source = LinkSource::PlatformConnector("discord".to_owned());
    let label = source.as_str();
    assert_eq!(label, "PlatformConnector(discord)");
    assert_eq!(label.parse::<LinkSource>().unwrap(), source);

    // The prefix is case-insensitive, matching the other variants' parse, and the underscored
    // agent-facing Lua label parses back too.
    assert_eq!(
        "platformconnector(discord)".parse::<LinkSource>().unwrap(),
        LinkSource::PlatformConnector("discord".to_owned())
    );
    assert_eq!(
        source.as_str_lowercase().parse::<LinkSource>().unwrap(),
        source
    );

    // A bare `PlatformConnector` with no parenthesised identifier is not a valid label.
    assert!("PlatformConnector".parse::<LinkSource>().is_err());
    assert!("PlatformConnectorish".parse::<LinkSource>().is_err());
    assert!("platform_connector".parse::<LinkSource>().is_err());
}
