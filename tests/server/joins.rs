use super::*;
#[tokio::test]
async fn a_restart_past_the_idle_gap_flushes_and_reopens() {
    let clock = ManualClock::new(test_now());
    let leads = ConversationLocator::new(TEST_PLATFORM, "leads");
    let model = ScriptedModel::new([
        Completion::Reply("one".to_owned()),
        Completion::Reply("two".to_owned()),
        // The recovery close flushes the lapsed session — its four turns meet flush_min_turns.
        Completion::Reply("flushed".to_owned()),
        Completion::Reply("three".to_owned()),
    ]);

    // First process: two messages — four turns, enough to trigger the flush on close. Its whole log is
    // snapshotted before the instance drops.
    let log = {
        let mut server = Server::new(
            Box::new(MemoryStore::new()),
            Graph::open_in_memory().unwrap(),
            Box::new(clock.clone()),
        );
        server.boot().unwrap();
        server.control().create_agent(&seed()).unwrap();
        server
            .platform()
            .route_message(
                &model,
                &leads,
                &PersonId::new(TEST_PLATFORM, "dave"),
                "hi",
                &[PersonId::new(TEST_PLATFORM, "dave")],
            )
            .await
            .unwrap();
        server
            .platform()
            .route_message(
                &model,
                &leads,
                &PersonId::new(TEST_PLATFORM, "dave"),
                "still here",
                &[PersonId::new(TEST_PLATFORM, "dave")],
            )
            .await
            .unwrap();
        assert_eq!(server.control().sessions(&leads).unwrap().len(), 1);
        server.control().events().unwrap()
    }; // restart — only the log survives, carried in memory

    // Second process: a fresh instance over the *same log* (carried in memory, no temp file). Past the
    // idle gap, the next message closes the recovered session (flushing its working state) and opens a
    // fresh one.
    let mut server = Server::new(
        Box::new(MemoryStore::from_events(log)),
        Graph::open_in_memory().unwrap(),
        Box::new(clock.clone()),
    );
    server.boot().unwrap();
    advance_past_idle_gap(&server, &clock);
    server
        .platform()
        .route_message(
            &model,
            &leads,
            &PersonId::new(TEST_PLATFORM, "dave"),
            "back again",
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();
    assert_eq!(
        server.control().sessions(&leads).unwrap().len(),
        2,
        "the stale recovered session closed and a fresh one opened"
    );
}

#[tokio::test]
async fn note_join_records_the_arriving_participant_on_the_session() {
    let (server, _clock) = born_agent();
    let model = ScriptedModel::new([Completion::Reply("hi".to_owned())]);
    let leads = ConversationLocator::new(TEST_PLATFORM, "leads");

    // Open a session with Dave present.
    server
        .platform()
        .route_message(
            &model,
            &leads,
            &PersonId::new(TEST_PLATFORM, "dave"),
            "hi",
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();
    let dave = server
        .control()
        .memory("person/dave@chat")
        .unwrap()
        .unwrap()
        .id;

    // Erin joins mid-session via the explicit endpoint path — with no model configured, so the
    // join-brief composes off the current prose rather than failing.
    server
        .platform()
        .note_join(None, &leads, &PersonId::new(TEST_PLATFORM, "erin"))
        .await
        .unwrap();
    let erin = server
        .control()
        .memory("person/erin@chat")
        .unwrap()
        .unwrap()
        .id;

    let sessions = server.control().sessions(&leads).unwrap();
    assert_eq!(sessions.len(), 1);
    let participants = &sessions[0].participants;
    assert!(participants.contains(&dave));
    assert!(participants.contains(&erin));

    // The endpoint shares the per-message sync's code path: the same join-brief system turn lands.
    let events = server.control().events().unwrap();
    assert!(
        events.iter().any(|event| matches!(
            &event.payload,
            EventPayload::ConversationTurn {
                role: TurnRole::System,
                participant: Some(participant),
                ..
            } if *participant == erin
        )),
        "note_join injects the same join-brief as the per-message sync"
    );
}

#[tokio::test]
async fn a_roster_resync_briefs_in_arrivals_and_leaves_departures_eventless() {
    let (server, _clock) = born_agent();
    let model = ScriptedModel::new([Completion::Reply("hi".to_owned())]);
    let leads = ConversationLocator::new(TEST_PLATFORM, "leads");

    // Open a session with Dave present.
    server
        .platform()
        .route_message(
            &model,
            &leads,
            &PersonId::new(TEST_PLATFORM, "dave"),
            "hi",
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();
    let dave = server
        .control()
        .memory("person/dave@chat")
        .unwrap()
        .unwrap()
        .id;

    // A roster resync reports Dave (already a member) and Erin (new): Erin arrives, Dave is
    // unchanged, and nobody has departed.
    let resync = server
        .platform()
        .note_presence(
            None,
            &leads,
            &[
                PersonId::new(TEST_PLATFORM, "dave"),
                PersonId::new(TEST_PLATFORM, "erin"),
            ],
        )
        .await
        .unwrap();
    assert_eq!(resync.joined, vec![PersonId::new(TEST_PLATFORM, "erin")]);
    assert_eq!(resync.departed, 0);
    let erin = server
        .control()
        .memory("person/erin@chat")
        .unwrap()
        .unwrap()
        .id;

    // Erin was briefed in through the same path as an explicit join: a system join-brief turn about
    // her, and she is now a session member.
    let events = server.control().events().unwrap();
    assert!(events.iter().any(|event| matches!(
        &event.payload,
        EventPayload::ConversationTurn {
            role: TurnRole::System,
            participant: Some(participant),
            ..
        } if *participant == erin
    )));
    let sessions = server.control().sessions(&leads).unwrap();
    assert!(sessions[0].participants.contains(&dave));
    assert!(sessions[0].participants.contains(&erin));

    // A resync that no longer lists Erin reports her departure — but records no event: her
    // membership on the session is unchanged (departures are eventless), and the log is untouched.
    let before = server.control().events().unwrap().len();
    let resync = server
        .platform()
        .note_presence(None, &leads, &[PersonId::new(TEST_PLATFORM, "dave")])
        .await
        .unwrap();
    assert!(resync.joined.is_empty());
    assert_eq!(resync.departed, 1);
    assert_eq!(
        server.control().events().unwrap().len(),
        before,
        "a departure appends no event"
    );
    assert!(
        server.control().sessions(&leads).unwrap()[0]
            .participants
            .contains(&erin),
        "the departed member's session membership is unchanged"
    );
}

#[tokio::test]
async fn a_roster_resync_both_adds_and_removes_in_one_call() {
    let (server, _clock) = born_agent();
    let model = ScriptedModel::new([Completion::Reply("hi".to_owned())]);
    let leads = ConversationLocator::new(TEST_PLATFORM, "leads");

    // Dave opens the session, then Erin joins via a resync.
    server
        .platform()
        .route_message(
            &model,
            &leads,
            &PersonId::new(TEST_PLATFORM, "dave"),
            "hi",
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();
    server
        .platform()
        .note_presence(
            None,
            &leads,
            &[
                PersonId::new(TEST_PLATFORM, "dave"),
                PersonId::new(TEST_PLATFORM, "erin"),
            ],
        )
        .await
        .unwrap();

    // A single resync in which Erin has left and Frank has arrived: one join, one departure.
    let resync = server
        .platform()
        .note_presence(
            None,
            &leads,
            &[
                PersonId::new(TEST_PLATFORM, "dave"),
                PersonId::new(TEST_PLATFORM, "frank"),
            ],
        )
        .await
        .unwrap();
    assert_eq!(resync.joined, vec![PersonId::new(TEST_PLATFORM, "frank")]);
    assert_eq!(resync.departed, 1);

    let frank = server
        .control()
        .memory("person/frank@chat")
        .unwrap()
        .unwrap()
        .id;
    assert!(
        server.control().sessions(&leads).unwrap()[0]
            .participants
            .contains(&frank)
    );
}

#[tokio::test]
async fn a_roster_resync_does_not_re_brief_existing_members() {
    let (server, _clock) = born_agent();
    let model = ScriptedModel::new([Completion::Reply("hi".to_owned())]);
    let leads = ConversationLocator::new(TEST_PLATFORM, "leads");

    server
        .platform()
        .route_message(
            &model,
            &leads,
            &PersonId::new(TEST_PLATFORM, "dave"),
            "hi",
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();

    // Resyncing the exact roster that is already present is a no-op: no arrival, no departure, and no
    // event appended.
    let before = server.control().events().unwrap().len();
    let resync = server
        .platform()
        .note_presence(None, &leads, &[PersonId::new(TEST_PLATFORM, "dave")])
        .await
        .unwrap();
    assert!(resync.joined.is_empty());
    assert_eq!(resync.departed, 0);
    assert_eq!(server.control().events().unwrap().len(), before);
}

#[tokio::test]
async fn an_empty_roster_departs_every_member_eventlessly() {
    let (server, _clock) = born_agent();
    let model = ScriptedModel::new([Completion::Reply("hi".to_owned())]);
    let leads = ConversationLocator::new(TEST_PLATFORM, "leads");

    server
        .platform()
        .route_message(
            &model,
            &leads,
            &PersonId::new(TEST_PLATFORM, "dave"),
            "hi",
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();

    let before = server.control().events().unwrap().len();
    let resync = server
        .platform()
        .note_presence(None, &leads, &[])
        .await
        .unwrap();
    assert!(resync.joined.is_empty());
    assert_eq!(
        resync.departed, 1,
        "Dave is the sole member, counted departed"
    );
    assert_eq!(server.control().events().unwrap().len(), before);
}

#[tokio::test]
async fn a_roster_resync_on_a_room_with_no_live_session_is_a_no_op() {
    let (server, _clock) = born_agent();
    let leads = ConversationLocator::new(TEST_PLATFORM, "leads");

    // No message has opened a session in this room, so there is nothing to resync: an empty result,
    // and no participant is minted or joined.
    let before = server.control().events().unwrap().len();
    let resync = server
        .platform()
        .note_presence(
            None,
            &leads,
            &[
                PersonId::new(TEST_PLATFORM, "dave"),
                PersonId::new(TEST_PLATFORM, "erin"),
            ],
        )
        .await
        .unwrap();
    assert!(resync.joined.is_empty());
    assert_eq!(resync.departed, 0);
    assert_eq!(server.control().events().unwrap().len(), before);
}

#[tokio::test]
async fn a_newcomers_first_mid_session_message_injects_a_join_brief_before_their_turn() {
    let (server, _clock) = born_agent();
    let leads = ConversationLocator::new(TEST_PLATFORM, "leads");
    let model = ScriptedModel::new([
        Completion::Reply("hi dave".to_owned()),
        Completion::Reply("hi erin".to_owned()),
    ]);

    // Dave opens the session alone; Erin's first message arrives mid-session, with no explicit
    // /platform/join posted — the message itself is the join signal.
    server
        .platform()
        .route_message(
            &model,
            &leads,
            &PersonId::new(TEST_PLATFORM, "dave"),
            "morning",
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();
    server
        .platform()
        .route_message(
            &model,
            &leads,
            &PersonId::new(TEST_PLATFORM, "erin"),
            "hey, joining in",
            &[
                PersonId::new(TEST_PLATFORM, "dave"),
                PersonId::new(TEST_PLATFORM, "erin"),
            ],
        )
        .await
        .unwrap();

    let erin = server
        .control()
        .memory("person/erin@chat")
        .unwrap()
        .unwrap()
        .id;
    let events = server.control().events().unwrap();

    // Exactly one join was recorded for the newcomer.
    let joins = events
        .iter()
        .filter(|event| {
            matches!(
                &event.payload,
                EventPayload::ParticipantJoined { participant, .. } if *participant == erin
            )
        })
        .count();
    assert_eq!(joins, 1, "one ParticipantJoined for the newcomer");

    // The injected join-brief — a system turn about Erin — precedes her inbound turn in the log,
    // and its text reflects her memory.
    let (brief_seq, brief_text) = events
        .iter()
        .find_map(|event| match &event.payload {
            EventPayload::ConversationTurn {
                role: TurnRole::System,
                participant: Some(participant),
                text,
                ..
            } if *participant == erin => Some((event.seq, text.clone())),
            _ => None,
        })
        .expect("the join injected a system join-brief turn");
    let inbound_seq = events
        .iter()
        .find_map(|event| match &event.payload {
            EventPayload::ConversationTurn {
                role: TurnRole::Participant,
                participant: Some(participant),
                ..
            } if *participant == erin => Some(event.seq),
            _ => None,
        })
        .expect("Erin's inbound turn is in the log");
    assert!(
        brief_seq.0 < inbound_seq.0,
        "the join-brief precedes the joiner's inbound turn"
    );
    assert!(
        brief_text.contains("person/erin@chat"),
        "the join-brief reflects the joiner's memory: {brief_text}"
    );

    // The join reused the live session, whose participants now include both.
    let dave = server
        .control()
        .memory("person/dave@chat")
        .unwrap()
        .unwrap()
        .id;
    let sessions = server.control().sessions(&leads).unwrap();
    assert_eq!(sessions.len(), 1, "the join reused the live session");
    assert!(sessions[0].participants.contains(&dave));
    assert!(sessions[0].participants.contains(&erin));
}

#[tokio::test]
async fn a_participant_merely_present_on_a_message_is_joined_too() {
    let (server, _clock) = born_agent();
    let leads = ConversationLocator::new(TEST_PLATFORM, "leads");
    let model = ScriptedModel::new([
        Completion::Reply("hi dave".to_owned()),
        Completion::Reply("noted".to_owned()),
    ]);

    server
        .platform()
        .route_message(
            &model,
            &leads,
            &PersonId::new(TEST_PLATFORM, "dave"),
            "morning",
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();
    // Erin never speaks — she only appears in Dave's present list — yet the presence sync joins her.
    server
        .platform()
        .route_message(
            &model,
            &leads,
            &PersonId::new(TEST_PLATFORM, "dave"),
            "erin just walked in",
            &[
                PersonId::new(TEST_PLATFORM, "dave"),
                PersonId::new(TEST_PLATFORM, "erin"),
            ],
        )
        .await
        .unwrap();

    let erin = server
        .control()
        .memory("person/erin@chat")
        .unwrap()
        .unwrap()
        .id;
    let events = server.control().events().unwrap();
    assert!(
        events.iter().any(|event| matches!(
            &event.payload,
            EventPayload::ParticipantJoined { participant, .. } if *participant == erin
        )),
        "presence alone records the join"
    );
    let sessions = server.control().sessions(&leads).unwrap();
    assert!(sessions[0].participants.contains(&erin));
}

#[tokio::test]
async fn repeat_messages_from_the_same_joiner_do_not_re_join() {
    let (server, _clock) = born_agent();
    let leads = ConversationLocator::new(TEST_PLATFORM, "leads");
    let model = ScriptedModel::new([
        Completion::Reply("hi dave".to_owned()),
        Completion::Reply("hi erin".to_owned()),
        Completion::Reply("still here".to_owned()),
    ]);

    server
        .platform()
        .route_message(
            &model,
            &leads,
            &PersonId::new(TEST_PLATFORM, "dave"),
            "morning",
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();
    server
        .platform()
        .route_message(
            &model,
            &leads,
            &PersonId::new(TEST_PLATFORM, "erin"),
            "hi",
            &[
                PersonId::new(TEST_PLATFORM, "dave"),
                PersonId::new(TEST_PLATFORM, "erin"),
            ],
        )
        .await
        .unwrap();
    server
        .platform()
        .route_message(
            &model,
            &leads,
            &PersonId::new(TEST_PLATFORM, "erin"),
            "one more thing",
            &[
                PersonId::new(TEST_PLATFORM, "dave"),
                PersonId::new(TEST_PLATFORM, "erin"),
            ],
        )
        .await
        .unwrap();

    let erin = server
        .control()
        .memory("person/erin@chat")
        .unwrap()
        .unwrap()
        .id;
    let events = server.control().events().unwrap();
    let joins = events
        .iter()
        .filter(|event| {
            matches!(
                &event.payload,
                EventPayload::ParticipantJoined { participant, .. } if *participant == erin
            )
        })
        .count();
    assert_eq!(joins, 1, "a member's later messages do not re-join");
}

#[tokio::test]
async fn a_due_wakeup_is_drained_into_the_next_eligible_session() {
    let (server, clock) = born_agent();
    let leads = ConversationLocator::new(TEST_PLATFORM, "leads");

    // Turn 1: the agent records a note on Dave's memory and the turn-end synthesis dates it to
    // 2026-07-01 — a calendared item scheduled weeks after the present test_now().
    let plant = ScriptedModel::new([
        run_lua_call(
            r#"memory.get("person/dave@chat"):append("dentist cleaning", { by_agent = true, visibility = "public" })"#,
        ),
        Completion::Reply("noted".to_owned()),
        Completion::Reply(
            serde_json::json!({
                "description": "Dave.",
                "occurrences": [{ "entry": 1, "occurred_at": { "day": "2026-07-01" } }],
            })
            .to_string(),
        ),
    ]);
    server
        .platform()
        .route_message(
            &plant,
            &leads,
            &PersonId::new(TEST_PLATFORM, "dave"),
            "remind me about the dentist",
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();

    // Temporal extraction runs off the hot path; drive the catch-up so the calendared item is
    // scheduled before the clock advances past it.
    server.describe_catch_up(&plant).await.unwrap();

    // Advance past the occurrence and the idle gap, so the next message opens a fresh session.
    clock.advance_millis(30 * MILLIS_PER_DAY);

    // Turn 2: opening this session fires the now-due wake-up and drains it as a system turn the agent
    // sees in its buffer.
    let drained = ScriptedModel::new([Completion::Reply("sure".to_owned())]);
    server
        .platform()
        .route_message(
            &drained,
            &leads,
            &PersonId::new(TEST_PLATFORM, "dave"),
            "what's up",
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();
    assert!(
        drained
            .recorded_messages()
            .iter()
            .flatten()
            .any(|message| message.content.contains("have come due")),
        "the drain should reach the model: {:?}",
        drained.recorded_messages()
    );

    // A later session, opened past the idle gap: the item was surfaced in the prior session, so the
    // scheduler never raises it a second time. The reopened session is seeded from the prior session's
    // tail (issue #86), so its buffer may still *carry* the earlier "have come due" system turn as
    // conversational history — that is the tail faithfully replaying what was said, not a fresh drain.
    // The structural check is the surfacing count: the item is surfaced exactly once across the log.
    clock.advance_millis(2 * MILLIS_PER_DAY);
    let quiet = ScriptedModel::new([Completion::Reply("ok".to_owned())]);
    server
        .platform()
        .route_message(
            &quiet,
            &leads,
            &PersonId::new(TEST_PLATFORM, "dave"),
            "still here",
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();
    let surfacings = server
        .control()
        .events()
        .unwrap()
        .iter()
        .filter(|event| matches!(event.payload, EventPayload::ScheduledItemSurfaced { .. }))
        .count();
    assert_eq!(
        surfacings, 1,
        "a surfaced item must not be raised a second time by the scheduler",
    );
}

#[tokio::test]
async fn a_token_budget_crossing_forces_a_re_segment_within_the_idle_gap() {
    let (server, _clock) = born_agent();
    // A tight token budget, so a single reported usage crosses it.
    let mut settings = server.control().settings().unwrap();
    settings.compaction.token_budget = 100;
    server.control().set_settings(settings).unwrap();

    let leads = ConversationLocator::new(TEST_PLATFORM, "leads");
    // Turn 1 reports usage over the budget; turn 2 is well under. Both arrive within the idle gap, so
    // only the token trigger — not idle — can force a second session.
    let model = ScriptedModel::with_usage([
        (Completion::Reply("one".to_owned()), 200),
        (Completion::Reply("two".to_owned()), 10),
    ]);

    server
        .platform()
        .route_message(
            &model,
            &leads,
            &PersonId::new(TEST_PLATFORM, "dave"),
            "hi",
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();
    assert_eq!(server.control().sessions(&leads).unwrap().len(), 1);

    server
        .platform()
        .route_message(
            &model,
            &leads,
            &PersonId::new(TEST_PLATFORM, "dave"),
            "still here",
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();
    let sessions = server.control().sessions(&leads).unwrap();
    assert_eq!(sessions.len(), 2);
    // The first session opened fresh; the re-segmented one carries a tail and re-freezes a brief.
    assert!(sessions[0].seeded_from_turn.is_none());
    assert!(sessions[1].seeded_from_turn.is_some());
    assert!(!sessions[1].brief.is_empty());
}

#[tokio::test]
async fn the_live_buffer_is_replayed_to_the_model_on_later_turns() {
    let (server, _clock) = born_agent();
    let leads = ConversationLocator::new(TEST_PLATFORM, "leads");
    let model = ScriptedModel::new([
        Completion::Reply("first reply".to_owned()),
        Completion::Reply("second reply".to_owned()),
    ]);

    server
        .platform()
        .route_message(
            &model,
            &leads,
            &PersonId::new(TEST_PLATFORM, "dave"),
            "hello there",
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();
    server
        .platform()
        .route_message(
            &model,
            &leads,
            &PersonId::new(TEST_PLATFORM, "dave"),
            "and again",
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();

    let seen = model.recorded_messages();
    assert_eq!(seen.len(), 2);
    // Turn 1's prompt is just the inbound message, stamped with who spoke and the time it was recorded
    // (test_now(); the clock does not advance in this test). The agent reads it, so it carries a
    // speaker-and-time prefix that lets it attribute the turn in a multi-party room.
    let turn1: Vec<&str> = seen[0]
        .iter()
        .map(|message| message.content.as_str())
        .collect();
    assert_eq!(turn1, vec!["[Mon 2026-06-08 00:00 UTC] dave: hello there"]);
    // Turn 2 replays the live buffer — turn 1's participant and agent turns — then the new inbound.
    // The participant turns it reads are speaker-and-time-stamped; the agent's own reply is left
    // unstamped (its `assistant` role already identifies it).
    let turn2: Vec<&str> = seen[1]
        .iter()
        .map(|message| message.content.as_str())
        .collect();
    assert_eq!(
        turn2,
        vec![
            "[Mon 2026-06-08 00:00 UTC] dave: hello there",
            "first reply",
            "[Mon 2026-06-08 00:00 UTC] dave: and again",
        ]
    );
}
