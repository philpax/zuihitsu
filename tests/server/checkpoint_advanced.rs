use super::*;
#[tokio::test]
async fn a_checkpointed_memory_is_retrievable_in_another_room() {
    let (server, _clock) = born_agent();
    tune_checkpoint(&server, 30, 0);

    let room_a = ConversationLocator::new("discord", "room-a");
    let room_b = ConversationLocator::new("discord", "room-b");
    let model = ScriptedModel::new([
        Completion::Reply("noted".to_owned()),
        Completion::Reply("hi erin".to_owned()),
        // The checkpoint flush in room A writes the decision to memory, mid-session.
        run_lua_call(
            r#"memory.create("topic/friday-launch", "Decided to ship the migration on Friday")"#,
        ),
        Completion::Reply("flushed".to_owned()),
        // Room B's next turn reads it back — the cross-conversation sync the checkpoint exists for.
        run_lua_call(r#"return memory.get("topic/friday-launch"):entries()"#),
        Completion::Reply("Dave's room decided to ship the migration on Friday.".to_owned()),
    ]);
    server
        .platform()
        .route_message(&model, &room_a, "dave", SUBSTANTIVE, &["dave"])
        .await
        .unwrap();
    server
        .platform()
        .route_message(&model, &room_b, "erin", "hello", &["erin"])
        .await
        .unwrap();
    assert_eq!(
        server
            .checkpoint_live_sessions(&model, CheckpointTrigger::Timer)
            .await
            .unwrap(),
        1
    );

    // Room A's session is still open (mid-conversation), yet its working state is already durable.
    assert!(
        server
            .control()
            .memory("topic/friday-launch")
            .unwrap()
            .is_some()
    );

    server
        .platform()
        .route_message(
            &model,
            &room_b,
            "erin",
            "what did dave's room decide?",
            &["erin"],
        )
        .await
        .unwrap();
    // Room B's read block saw the checkpointed content, before room A ever went idle.
    let events = server.control().events().unwrap();
    let read_back = events.iter().any(|event| {
        matches!(
            &event.payload,
            EventPayload::LuaExecuted { script, result: Some(result), .. }
                if script.contains("friday-launch") && result.contains("Friday")
        )
    });
    assert!(
        read_back,
        "room B's next turn should read the checkpointed memory"
    );
}

/// A model that answers from a script but parks one designated call until released — the window a
/// concurrency test opens to overlap other work with an in-flight flush.
struct GatedModel {
    completions: Mutex<std::collections::VecDeque<Completion>>,
    calls: AtomicUsize,
    gated_call: usize,
    entered: std::sync::atomic::AtomicBool,
    release: tokio::sync::Notify,
}

impl GatedModel {
    fn new(completions: impl IntoIterator<Item = Completion>, gated_call: usize) -> GatedModel {
        GatedModel {
            completions: Mutex::new(completions.into_iter().collect()),
            calls: AtomicUsize::new(0),
            gated_call,
            entered: std::sync::atomic::AtomicBool::new(false),
            release: tokio::sync::Notify::new(),
        }
    }

    /// Whether the gated call has been reached (and is parked).
    fn entered(&self) -> bool {
        self.entered.load(Ordering::SeqCst)
    }

    /// Let the parked call proceed.
    fn release(&self) {
        self.release.notify_one();
    }
}

#[async_trait::async_trait]
impl ModelClient for GatedModel {
    fn model_id(&self) -> &str {
        "gated-model"
    }

    async fn generate_stream(&self, _request: &GenerateRequest) -> GenerateStream {
        let step: Result<GenerateResponse, ModelError> = async {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            if call == self.gated_call {
                self.entered.store(true, Ordering::SeqCst);
                // `notify_one` before this await stores a permit, so a release observed via `entered`
                // can never be missed.
                self.release.notified().await;
            }
            let completion = self
                .completions
                .lock()
                .unwrap()
                .pop_front()
                .expect("a scripted completion for every call");
            Ok(GenerateResponse {
                completion,
                usage: Usage::default(),
                reasoning: None,
                finish_reason: None,
            })
        }
        .await;
        stream_response(step)
    }
}

#[tokio::test]
async fn an_arriving_message_waits_for_an_in_flight_checkpoint_flush() {
    let (server, _clock) = born_agent();
    tune_checkpoint(&server, 30, 0);
    let server = Arc::new(server);

    let room_a = ConversationLocator::new("discord", "room-a");
    let room_b = ConversationLocator::new("discord", "room-b");
    // Call 2 — the checkpoint flush — parks until released; the message that arrives while it is
    // parked must wait on the conversation's lifecycle lock rather than interleave.
    let model = Arc::new(GatedModel::new(
        [
            Completion::Reply("noted".to_owned()),
            Completion::Reply("hi".to_owned()),
            Completion::Reply("checkpointed".to_owned()),
            Completion::Reply("welcome back".to_owned()),
        ],
        2,
    ));
    server
        .platform()
        .route_message(model.as_ref(), &room_a, "dave", SUBSTANTIVE, &["dave"])
        .await
        .unwrap();
    server
        .platform()
        .route_message(model.as_ref(), &room_b, "erin", "hello", &["erin"])
        .await
        .unwrap();

    // Start the sweep; it takes room A's lifecycle lock and parks inside the flush's model call.
    let sweep = tokio::spawn({
        let server = server.clone();
        let model = model.clone();
        async move {
            server
                .checkpoint_live_sessions(model.as_ref(), CheckpointTrigger::Timer)
                .await
        }
    });
    while !model.entered() {
        tokio::task::yield_now().await;
    }

    // A message arrives mid-flush. It must queue behind the flush in `ensure_session` — its inbound
    // turn cannot land while the flush holds the lock.
    let message = tokio::spawn({
        let server = server.clone();
        let model = model.clone();
        async move {
            server
                .platform()
                .route_message(
                    model.as_ref(),
                    &ConversationLocator::new("discord", "room-a"),
                    "dave",
                    "one more thing",
                    &["dave"],
                )
                .await
        }
    });
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
    let mid_flush = server.control().events().unwrap();
    assert!(
        !mid_flush.iter().any(|event| {
            matches!(&event.payload, EventPayload::ConversationTurn { text, .. } if text == "one more thing")
        }),
        "the arriving message must wait on the lifecycle lock while the flush is in flight"
    );

    model.release();
    assert_eq!(sweep.await.unwrap().unwrap(), 1);
    message.await.unwrap().unwrap();

    // Serialized through the lock: exactly one flush turn, recorded before the waiting message's
    // inbound turn — no double flush, and no interleaving.
    let events = server.control().events().unwrap();
    let flush_seqs: Vec<_> = events
        .iter()
        .filter(|event| {
            matches!(
                &event.payload,
                EventPayload::ConversationTurn { produced_by: Some(produced), .. }
                    if produced.template_name == PromptTemplateName::Flush
            )
        })
        .map(|event| event.seq)
        .collect();
    assert_eq!(flush_seqs.len(), 1, "exactly one flush turn is recorded");
    let inbound_seq = events
        .iter()
        .find(|event| {
            matches!(&event.payload, EventPayload::ConversationTurn { text, .. } if text == "one more thing")
        })
        .map(|event| event.seq)
        .expect("the waiting message lands after the flush releases");
    assert!(
        flush_seqs[0] < inbound_seq,
        "the flush turn precedes the message that waited on it"
    );
}

#[tokio::test]
async fn context_current_resolves_during_a_routed_turn() {
    let (server, _clock) = born_agent();
    let leads = ConversationLocator::new("discord", "leads");
    // The agent appends to the current context. If context.current() returned nil in the routed path
    // (as a real-model run's stray `Context: nil` print suggested), this would error on nil:append
    // and commit nothing.
    let model = ScriptedModel::new([
        run_lua_call(r#"context.current():append("a note in the room", { by_agent = true })"#),
        Completion::Reply("noted".to_owned()),
    ]);
    server
        .platform()
        .route_message(&model, &leads, "dave", "hi", &["dave"])
        .await
        .unwrap();
    // The context memory received the entry — context.current() resolved through route_message.
    let entries = server.control().entries("context/discord:leads").unwrap();
    assert!(
        entries
            .iter()
            .any(|entry| entry.text == "a note in the room"),
        "context entries: {entries:?}"
    );
}

#[tokio::test]
async fn the_working_set_carries_into_the_next_session_brief() {
    let (server, _clock) = born_agent();
    let mut settings = server.control().settings().unwrap();
    settings.compaction.token_budget = 100;
    server.control().set_settings(settings).unwrap();

    let leads = ConversationLocator::new("discord", "leads");
    let model = ScriptedModel::with_usage([
        // Turn 1 touches a memory, then crosses the budget (two turns — below the flush gate).
        (
            run_lua_call(r#"memory.create("topic/roadmap", "Plan the Q3 work")"#),
            10,
        ),
        (Completion::Reply("on it".to_owned()), 200),
        // Regeneration of the touched memory's description.
        (describe_call("The team's Q3 roadmap."), 0),
        // Turn 2 opens the re-segmented session; its frozen brief is what we inspect.
        (Completion::Reply("hello again".to_owned()), 0),
    ]);

    server
        .platform()
        .route_message(&model, &leads, "dave", "let's plan", &["dave"])
        .await
        .unwrap();
    server
        .platform()
        .route_message(&model, &leads, "dave", "back", &["dave"])
        .await
        .unwrap();

    let sessions = server.control().sessions(&leads).unwrap();
    assert_eq!(sessions.len(), 2);
    // The re-segmented session's brief re-surfaces the touched memory as an active thread.
    let brief = &sessions[1].brief;
    assert!(brief.contains("# Active threads"), "brief was: {brief}");
    assert!(brief.contains("topic/roadmap"), "brief was: {brief}");
}
