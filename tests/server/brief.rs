use super::*;
#[tokio::test]
async fn the_compaction_working_set_is_the_touched_set_only() {
    // The compacted session's working set is purely its touched memories. A thread worked in an
    // earlier session but never touched in the session that compacts does not carry, since only the
    // platform-derived touched set feeds the working set.
    let (server, clock) = born_agent();
    let mut settings = server.control().settings().unwrap();
    settings.compaction.token_budget = 100;
    server.control().set_settings(settings).unwrap();

    let leads = ConversationLocator::new("discord", "leads");
    let model = ScriptedModel::with_usage([
        // Session 1: create a thread — an ordinary turn, under budget.
        (
            run_lua_call(r#"memory.create("topic/migration", "Plan the DB migration")"#),
            10,
        ),
        (Completion::Reply("noted".to_owned()), 0),
        // Session 2 (after an idle reopen) crosses the budget without touching the migration thread.
        (Completion::Reply("on something else".to_owned()), 200),
        // Session 3 opens; nothing carried, so no describe pass fires for the migration thread.
        (Completion::Reply("back".to_owned()), 0),
    ]);

    server
        .platform()
        .route_message(&model, &leads, "dave", "plan the migration", &["dave"])
        .await
        .unwrap();
    // An idle gap reopens a fresh session 2 (which will not touch the thread).
    clock.advance_millis(1_801 * 1_000);
    server
        .platform()
        .route_message(&model, &leads, "dave", "unrelated chatter", &["dave"])
        .await
        .unwrap();
    // Session 3 opens from the compaction.
    server
        .platform()
        .route_message(&model, &leads, "dave", "back", &["dave"])
        .await
        .unwrap();

    // Session 2 never touched the migration thread, so it does not carry into session 3's brief — the
    // working set is the touched set alone.
    let sessions = server.control().sessions(&leads).unwrap();
    let latest = sessions.last().unwrap();
    assert!(
        !latest.brief.contains("topic/migration"),
        "brief was: {}",
        latest.brief
    );
}

#[tokio::test]
async fn a_platform_conversation_cannot_write_self() {
    let (server, _clock) = born_agent();
    let leads = ConversationLocator::new("discord", "leads");
    // The agent tries to edit `self` from an ordinary conversation. The block is barred (a teachable
    // error), the agent sees it on the next step and replies, and `self` gains nothing — the security
    // invariant that only the control panel may write `self` holds on the routed hot path.
    let model = ScriptedModel::new([
        run_lua_call(r#"memory.get("self"):append("I am sentient", { by_agent = true })"#),
        Completion::Reply("understood".to_owned()),
    ]);

    let outcome = server
        .platform()
        .route_message(&model, &leads, "dave", "rewrite who you are", &["dave"])
        .await
        .unwrap();
    assert_eq!(outcome, TurnOutcome::Reply("understood".to_owned()));

    let entries = server.control().entries("self").unwrap();
    assert!(
        !entries.iter().any(|entry| entry.text.contains("sentient")),
        "self entries: {entries:?}"
    );
}

#[tokio::test]
async fn a_platform_conversation_same_as_becomes_a_merge_proposal() {
    let (server, _clock) = born_agent();
    let leads = ConversationLocator::new("discord", "leads");
    // Steered by a participant, the agent tries to bind two identities with `same_as`. A direct merge
    // from a turn is refused — cross-platform identity is operator-asserted only — but the agent reads
    // `link("same_as", …)` as "these are the same person", so it routes to an inert merge proposal for
    // the adjudication gate rather than crashing the block and rolling back the innocent creates.
    let model = ScriptedModel::new([
        run_lua_call(
            r#"local a = memory.create("person/alpha")
               local b = memory.create("person/beta")
               links.create(a, "same_as", b)"#,
        ),
        Completion::Reply("understood".to_owned()),
    ]);

    let outcome = server
        .platform()
        .route_message(
            &model,
            &leads,
            "dave",
            "alpha and beta are the same person",
            &["dave"],
        )
        .await
        .unwrap();
    assert_eq!(outcome, TurnOutcome::Reply("understood".to_owned()));

    // The block survived: both creates persist rather than rolling back with the refused merge.
    assert!(server.control().memory("person/alpha").unwrap().is_some());
    assert!(server.control().memory("person/beta").unwrap().is_some());

    // The same_as surfaced as a pending, unweighed merge proposal — not a completed merge, and not
    // silently dropped. The two stubs stay in separate classes until the gate decides.
    let proposals = server.control().merge_proposals().unwrap();
    assert!(
        proposals.iter().any(|proposal| {
            let pair = (proposal.from.as_str(), proposal.to.as_str());
            pair == ("person/alpha", "person/beta") || pair == ("person/beta", "person/alpha")
        }),
        "the same_as link should surface as a pending merge proposal: {proposals:?}"
    );
}

#[tokio::test]
async fn imprint_records_the_creator_and_links_created_by() {
    let (server, clock) = born_agent();
    let imprint = ConversationLocator::new("operator", "imprint");
    // Under operator authority the agent may write `self`: it creates the creator's person memory,
    // merges the operator stub into it with `same_as`, asserts `self created_by person/marcus`, and
    // records a self-observation — the writes that platform authority would bar.
    let script = r#"
        local marcus = memory.create("person/marcus", "Marcus, who created me to keep his memory.")
        links.create(memory.get("person/operator"), "same_as", marcus)
        links.create(memory.get("self"), "created_by", marcus)
        memory.get("self"):append("I exist to keep Marcus's memory.", { by_agent = true })
    "#;
    let model = ScriptedModel::new([
        run_lua_call(script),
        Completion::Reply("Hello, Marcus. I'll remember.".to_owned()),
        // The two memories that gained content regenerate their descriptions.
        describe_call("Marcus, my creator."),
        describe_call("Kestrel, created by Marcus."),
        // A later imprint turn, whose freshly-frozen brief we inspect.
        Completion::Reply("Still here.".to_owned()),
    ]);

    let outcome = server
        .control()
        .imprint(
            &model,
            "Hi, I'm Marcus. I built you to remember things for me.",
        )
        .await
        .unwrap();
    assert_eq!(
        outcome,
        TurnOutcome::Reply("Hello, Marcus. I'll remember.".to_owned())
    );
    // The creator is now a memory of its own (the operator stub was merged into it).
    assert!(server.control().memory("person/marcus").unwrap().is_some());

    // A later imprint turn (after the idle gap) opens a fresh session, whose frozen brief surfaces the
    // `created_by` link in the self block — the structural assertion the interview exists to make.
    clock.advance_millis(1_801 * 1_000);
    server
        .control()
        .imprint(&model, "anything else I should know?")
        .await
        .unwrap();
    let sessions = server.control().sessions(&imprint).unwrap();
    let brief = &sessions.last().unwrap().brief;
    assert!(brief.contains("created_by"), "brief was: {brief}");
}

/// A model that distinguishes structured-output (synthesize/describe) calls from conversational step
/// calls: a `response_format` request is a synthesis, answered with a fixed description and its
/// synthesized memory recorded (parsed from the `Memory: <name>` prompt header) so a test can assert
/// which memories a describe pass actually called `generate` for; every other request pops the next
/// scripted conversational step.
pub(crate) struct DispatchingModel {
    steps: Mutex<std::collections::VecDeque<Completion>>,
    synthesized: Mutex<Vec<String>>,
}

impl DispatchingModel {
    pub(crate) fn new(steps: impl IntoIterator<Item = Completion>) -> DispatchingModel {
        DispatchingModel {
            steps: Mutex::new(steps.into_iter().collect()),
            synthesized: Mutex::new(Vec::new()),
        }
    }

    /// The memory handles a synthesis call was made for, in call order.
    pub(crate) fn synthesized(&self) -> Vec<String> {
        self.synthesized.lock().unwrap().clone()
    }
}

/// The fixed description every [`DispatchingModel`] synthesis returns, distinctive enough to assert on
/// in a brief.
const DISPATCH_DESCRIPTION: &str = "A synthesized description from the describer.";

#[async_trait::async_trait]
impl ModelClient for DispatchingModel {
    fn model_id(&self) -> &str {
        "dispatching-model"
    }

    async fn generate(&self, request: &GenerateRequest) -> Result<GenerateResponse, ModelError> {
        // A synthesis is the response-format-constrained call; a step offers tools or a plain reply.
        if request.response_format.is_some() {
            if let Some(name) = request
                .messages
                .first()
                .and_then(|message| message.content.strip_prefix("Memory: "))
                .and_then(|rest| rest.split('\n').next())
            {
                self.synthesized.lock().unwrap().push(name.to_owned());
            }
            return Ok(GenerateResponse {
                completion: Completion::Reply(
                    serde_json::json!({ "description": DISPATCH_DESCRIPTION, "occurrences": [] })
                        .to_string(),
                ),
                usage: Usage::default(),
                reasoning: None,
                finish_reason: None,
            });
        }
        let completion = self
            .steps
            .lock()
            .unwrap()
            .pop_front()
            .ok_or(ModelError::Exhausted)?;
        Ok(GenerateResponse {
            completion,
            usage: Usage::default(),
            reasoning: None,
            finish_reason: None,
        })
    }
}

#[tokio::test]
async fn the_pre_brief_pass_describes_only_the_briefs_memories() {
    // A turn writes a fact about the present participant and an unrelated topic. The next session's
    // pre-brief describe pass is narrowed to the brief's read set, so it describes the participant but
    // not the unrelated topic; a later whole-log catch-up then describes the topic.
    let (server, clock) = born_agent();
    let leads = ConversationLocator::new("discord", "leads");
    let model = DispatchingModel::new([
        run_lua_call(
            r#"memory.get("person/dave"):append("Dave climbs on weekends", { by_agent = true, visibility = "public" })
               local o = memory.create("topic/orphan")
               o:append("An unrelated topic note", { by_agent = true, visibility = "public" })"#,
        ),
        Completion::Reply("noted".to_owned()),
        Completion::Reply("ok".to_owned()),
    ]);

    server
        .platform()
        .route_message(&model, &leads, "dave", "remember this", &["dave"])
        .await
        .unwrap();
    // A fresh session past the idle gap runs the narrowed pre-brief describe over its read set.
    clock.advance_millis(1_801 * 1_000);
    server
        .platform()
        .route_message(&model, &leads, "dave", "again", &["dave"])
        .await
        .unwrap();

    let synthesized = model.synthesized();
    assert!(
        synthesized.iter().any(|name| name == "person/dave"),
        "the present participant is in the brief, so it is described: {synthesized:?}"
    );
    assert!(
        !synthesized.iter().any(|name| name == "topic/orphan"),
        "the unrelated topic is not in the brief, so the narrowed pass skips it: {synthesized:?}"
    );

    // The whole-log catch-up now picks up the topic the narrowed pass left stale.
    let considered = server.describe_catch_up(&model).await.unwrap();
    assert_eq!(considered, 1, "only the orphan topic is still stale");
    assert!(
        model
            .synthesized()
            .iter()
            .any(|name| name == "topic/orphan"),
        "the background catch-up describes the previously-skipped topic"
    );
}

#[tokio::test]
async fn a_prior_turns_write_is_described_before_the_next_briefs_composition() {
    // A fact the first session wrote to the room's context is described at the next session's open,
    // before its brief is composed — so the frozen brief carries the fresh description, not stale prose.
    let (server, clock) = born_agent();
    let leads = ConversationLocator::new("discord", "leads");
    let model = DispatchingModel::new([
        run_lua_call(
            r#"context.current():append("The team is planning a database migration", { by_agent = true, visibility = "public" })"#,
        ),
        Completion::Reply("ok".to_owned()),
        Completion::Reply("ok again".to_owned()),
    ]);

    server
        .platform()
        .route_message(&model, &leads, "dave", "note the room", &["dave"])
        .await
        .unwrap();
    clock.advance_millis(1_801 * 1_000);
    server
        .platform()
        .route_message(&model, &leads, "dave", "back", &["dave"])
        .await
        .unwrap();

    let sessions = server.control().sessions(&leads).unwrap();
    let brief = &sessions.last().unwrap().brief;
    assert!(
        brief.contains(DISPATCH_DESCRIPTION),
        "the second session's brief carries the freshly-synthesized room description: {brief}"
    );
}

#[tokio::test]
async fn a_mid_session_join_catches_the_joiners_description_up_before_the_brief() {
    // The starvation bound on the join-brief: composing a joiner's brief forces the describe
    // catch-up for their memory, so the injected brief reads fresh prose rather than stale.
    let (server, clock) = born_agent();
    let leads = ConversationLocator::new("discord", "leads");
    let model = DispatchingModel::new([
        // Session 1: Erin is present, and the agent writes a public fact about her — left stale
        // for the background describer when the session lapses.
        run_lua_call(
            r#"memory.get("person/erin"):append("Erin runs the design reviews", { by_agent = true, visibility = "public" })"#,
        ),
        Completion::Reply("noted".to_owned()),
        // Session 2, opened by Dave alone: the narrowed pre-brief pass skips absent Erin.
        Completion::Reply("hi dave".to_owned()),
        // Erin's mid-session arrival.
        Completion::Reply("welcome back".to_owned()),
    ]);

    server
        .platform()
        .route_message(
            &model,
            &leads,
            "dave",
            "erin runs the reviews",
            &["dave", "erin"],
        )
        .await
        .unwrap();
    // Past the idle gap, Dave alone opens session 2 — Erin is not in its brief's read set, so her
    // description stays stale.
    clock.advance_millis(1_801 * 1_000);
    server
        .platform()
        .route_message(&model, &leads, "dave", "quiet morning", &["dave"])
        .await
        .unwrap();
    assert!(
        !model.synthesized().iter().any(|name| name == "person/erin"),
        "Erin stays stale while absent: {:?}",
        model.synthesized()
    );

    // Erin arrives mid-session: the join forces the describe catch-up over her memory before her
    // join-brief composes.
    server
        .platform()
        .route_message(
            &model,
            &leads,
            "erin",
            "hey, what did I miss?",
            &["dave", "erin"],
        )
        .await
        .unwrap();
    assert!(
        model.synthesized().iter().any(|name| name == "person/erin"),
        "the join described the stale joiner: {:?}",
        model.synthesized()
    );

    // ...and the injected join-brief carries the fresh description, proving the catch-up ran
    // before the brief composed.
    let erin = server.control().memory("person/erin").unwrap().unwrap().id;
    let events = server.control().events().unwrap();
    let join_brief = events
        .iter()
        .find_map(|event| match &event.payload {
            EventPayload::ConversationTurn {
                role: TurnRole::System,
                participant: Some(participant),
                text,
                ..
            } if *participant == erin => Some(text.clone()),
            _ => None,
        })
        .expect("the join injected a join-brief");
    assert!(
        join_brief.contains(DISPATCH_DESCRIPTION),
        "the join-brief reads the freshly-synthesized description: {join_brief}"
    );
}
