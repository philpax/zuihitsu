use super::*;

/// Register the merge-adjudication template directly, so the adjudication pass has its prompt without a
/// full genesis rollout (the scripted model returns a fixed verdict regardless of the prompt text).
fn register_adjudication_template(h: &Harness) {
    h.engine
        .store
        .lock()
        .as_mut()
        .append(
            h.clock.now(),
            vec![EventPayload::prompt_template_registered(
                PromptTemplateName::MergeAdjudication,
                1,
                "Decide whether two stubs are the same person, on the evidence.".to_owned(),
                EventSource::Orchestration,
            )],
        )
        .unwrap();
}

#[tokio::test]
async fn an_adjudicated_merge_links_two_stubs_on_accept() {
    // The agent proposes two stubs are one person; the off-hot-path adjudicator, accepting, authors the
    // same_as that merges them into one class (spec §Cross-platform identity → adjudicated merge).
    let h = Harness::new();
    register_adjudication_template(&h);
    h.run(
        r#"
        local a = memory.create(PERSON_DAVE_SLACK)
        a:append("Off sick the first week of March", { visibility = "private" })
        local b = memory.create(PERSON_DAVE_DISCORD)
        b:append("Out sick the week of March 3rd", { visibility = "private" })
        a:propose_merge(b)
        return "ok"
        "#,
    )
    .await;

    let model = ScriptedModel::new([Completion::Reply(
        r#"{"accepted": true, "rationale": "Both off sick the same week — an improbable coincidence."}"#
            .to_owned(),
    )]);
    h.adjudicate(&model).await;

    let graph = h.engine.graph.lock();
    let a = graph
        .memory_by_name(Namespace::Person.with_name("dave-slack"))
        .unwrap()
        .unwrap();
    let b = graph
        .memory_by_name(Namespace::Person.with_name("dave-discord"))
        .unwrap()
        .unwrap();
    let members = graph.class_members(a.id).unwrap();
    assert!(
        members.contains(&b.id),
        "the accepted merge should put both stubs in one same_as class, got {members:?}"
    );
}

#[tokio::test]
async fn a_refused_merge_leaves_the_stubs_distinct() {
    // On only a generic overlap the adjudicator refuses; no same_as is authored, the stubs stay in
    // separate classes, and the refusal is recorded for the operator.
    let h = Harness::new();
    register_adjudication_template(&h);
    h.run(
        r#"
        local a = memory.create(PERSON_SAM_SLACK)
        a:append("Is an engineer", { visibility = "public" })
        local b = memory.create(PERSON_SAM_DISCORD)
        b:append("Works in engineering", { visibility = "public" })
        a:propose_merge(b)
        return "ok"
        "#,
    )
    .await;

    let model = ScriptedModel::new([Completion::Reply(
        r#"{"accepted": false, "rationale": "Only a generic overlap; no specific coincidence."}"#
            .to_owned(),
    )]);
    h.adjudicate(&model).await;

    let graph = h.engine.graph.lock();
    let a = graph
        .memory_by_name(Namespace::Person.with_name("sam-slack"))
        .unwrap()
        .unwrap();
    let b = graph
        .memory_by_name(Namespace::Person.with_name("sam-discord"))
        .unwrap()
        .unwrap();
    assert!(
        !graph.class_members(a.id).unwrap().contains(&b.id),
        "a refused merge must leave the stubs in separate classes"
    );
    drop(graph);
    let events = h.events();
    assert!(
        events.iter().any(|e| matches!(
            &e.payload,
            EventPayload::MergeAdjudicated {
                accepted: false,
                ..
            }
        )),
        "a refusing verdict should be recorded for the operator"
    );
}

#[tokio::test]
async fn a_proposals_rationale_reaches_the_adjudication_prompt() {
    // The rationale the agent states with propose_merge rides the MergeProposed event and is injected
    // into the adjudicator's prompt as the proposer's claim — so the adjudicator weighs the stated
    // grounds against the two stubs' persisted entries rather than seeing only the entries.
    let h = Harness::new();
    register_adjudication_template(&h);
    h.run(
        r#"
        local a = memory.create(PERSON_DAVE_SLACK)
        a:append("At the Reykjavik conference in June", { visibility = "public" })
        local b = memory.create(PERSON_DAVE_DISCORD)
        b:append("Was on a research trip to Iceland", { visibility = "public" })
        a:propose_merge(b, { rationale = "Both mention the same volcanology trip and the same wedding." })
        return "ok"
        "#,
    )
    .await;

    // The stated grounds ride the event.
    assert!(
        h.events().iter().any(|e| matches!(
            &e.payload,
            EventPayload::MergeProposed { rationale: Some(text), .. }
                if text == "Both mention the same volcanology trip and the same wedding."
        )),
        "the rationale must ride the MergeProposed event"
    );

    let model = ScriptedModel::new([Completion::Reply(
        r#"{"accepted": false, "rationale": "Weighed the claim against the facts; not enough."}"#
            .to_owned(),
    )]);
    h.adjudicate(&model).await;

    // ... and reach the adjudicator's prompt, labeled as the proposer's claim rather than as evidence.
    let prompts: Vec<String> = model
        .recorded_messages()
        .iter()
        .flatten()
        .map(|message| message.content.clone())
        .collect();
    assert!(
        prompts.iter().any(|p| {
            p.contains("Both mention the same volcanology trip and the same wedding.")
                && p.contains("their claim, not evidence")
        }),
        "the adjudication prompt must carry the proposer's stated rationale as a claim: {prompts:?}"
    );
}
