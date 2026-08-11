//! What the model is shown when a participant's message carries files, live and on replay.

use crate::{
    Completion, Harness, InstanceFeatures, PromptTemplateName, ScriptedModel, Seq, TurnRole,
    buffer_turns, genesis, run_turn, seed,
};

/// A harness with genesis rolled out and the graph materialized — what a turn needs to run.
fn ready() -> Harness {
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
    h
}

#[tokio::test]
async fn a_shared_image_reaches_the_model_as_a_content_part() {
    let mut h = ready();
    let image = h.attach("diagram.png", "image/png", b"\x89PNG\r\n\x1a\n");
    h.with_attachments(vec![image.clone()]);

    let model = ScriptedModel::new([Completion::Reply("A diagram.".to_owned())]);
    run_turn(h.as_turn(&model, "what is this", 8))
        .await
        .unwrap();

    let sent = model.recorded_messages();
    let user = sent[0]
        .iter()
        .rfind(|m| m.role == zuihitsu::Role::User)
        .expect("the inbound message");
    let [part] = &user.images[..] else {
        panic!("one image part, got {:?}", user.images);
    };
    assert_eq!(part.blob, image.blob);
    assert_eq!(part.mime, "image/png");
    assert_eq!(&*part.data, "iVBORw0KGgo=");
    // The body still says what was shared, so the text alone is not left dangling.
    assert!(
        user.content.contains("[attachment: diagram.png"),
        "{}",
        user.content
    );
}

#[tokio::test]
async fn a_shared_text_file_is_inlined_into_the_message() {
    let mut h = ready();
    let notes = h.attach("build.log", "text/plain", b"error: it broke");
    h.with_attachments(vec![notes]);

    let model = ScriptedModel::new([Completion::Reply("It broke.".to_owned())]);
    run_turn(h.as_turn(&model, "any idea?", 8)).await.unwrap();

    let sent = model.recorded_messages();
    let user = sent[0]
        .iter()
        .rfind(|m| m.role == zuihitsu::Role::User)
        .expect("the inbound message");
    assert!(user.content.contains("error: it broke"), "{}", user.content);
    assert!(user.images.is_empty());
}

#[tokio::test]
async fn a_replayed_turn_still_shows_the_model_the_image() {
    // The attachment is recorded on the participant turn, so the next turn's buffer replay must show
    // the model the same image — and show it identically, or the prefix cache is lost every turn.
    let mut h = ready();
    let image = h.attach("diagram.png", "image/png", b"\x89PNG\r\n\x1a\n");
    h.with_attachments(vec![image.clone()]);

    let first = ScriptedModel::new([Completion::Reply("A diagram.".to_owned())]);
    run_turn(h.as_turn(&first, "what is this", 8))
        .await
        .unwrap();

    let conversation = h.session.conversation().unwrap();
    let buffer = buffer_turns(h.engine.store.lock().as_ref(), conversation, Seq(0)).unwrap();
    let recorded = buffer
        .iter()
        .find(|turn| turn.role == TurnRole::Participant)
        .expect("the participant turn");
    assert_eq!(recorded.attachments, vec![image.clone()]);

    let second = ScriptedModel::new([Completion::Reply("Still a diagram.".to_owned())]);
    let mut turn = h.as_turn(&second, "and now?", 8);
    turn.buffer = &buffer;
    turn.template = PromptTemplateName::Scaffold;
    run_turn(turn).await.unwrap();

    let replayed = second.recorded_messages();
    let carried = replayed[0]
        .iter()
        .filter(|m| m.role == zuihitsu::Role::User)
        .find(|m| !m.images.is_empty())
        .expect("the replayed turn still carries its image");
    assert_eq!(carried.images[0].blob, image.blob);
    assert_eq!(&*carried.images[0].data, "iVBORw0KGgo=");

    // Byte-identical to what the live turn sent, modulo the speaker stamp the replay adds.
    let live = first.recorded_messages();
    let live_user = live[0]
        .iter()
        .rfind(|m| m.role == zuihitsu::Role::User)
        .expect("the live inbound message");
    assert!(
        carried.content.ends_with(
            live_user
                .content
                .split_once("] ")
                .map(|(_, rest)| rest)
                .unwrap_or(&live_user.content)
        ),
        "replayed {:?} does not reproduce live {:?}",
        carried.content,
        live_user.content
    );
}
