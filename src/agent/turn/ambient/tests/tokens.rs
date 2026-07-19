//! Tests for the turn-token lead: a `[turn:<id>]` reference leads the hint with a `convo.turn`
//! pointer, fires on its own, stays silent when transcripts are off, and caps at the first few.

use std::collections::HashSet;

use super::{corpus, topic};
use crate::{
    agent::turn::ambient::{MAX_TURN_TOKENS, ambient_recall},
    ids::{MemoryId, TurnId},
    settings::AmbientSettings,
    turn_ref,
};

#[test]
fn a_turn_token_leads_the_hint_and_fires_without_a_lexical_hit() {
    // A message that cites a recorded moment but matches nothing lexically still surfaces a hint:
    // the token line leads, pointing at convo.turn, so the reference is never inert.
    let bonsai = MemoryId::generate();
    let graph = corpus(topic(
        bonsai,
        "bonsai",
        "A schema-migration tool Erin built.",
    ));
    let turn = TurnId::generate();
    let message = format!(
        "Can you dig up what we said in {}?",
        turn_ref::construct(turn)
    );
    let hint = ambient_recall(
        &graph,
        &AmbientSettings::default(),
        &message,
        &HashSet::new(),
        true,
        true,
    )
    .unwrap()
    .expect("the token fires the hint even with no lexical hit");
    assert!(hint.hits.is_empty(), "no lexical hit rode along");
    let first = hint.message.lines().next().unwrap();
    assert!(
        first.contains(&format!("convo.turn(\"{}\")", turn.0)),
        "the hint leads with the token's resolver: {first}"
    );
}

#[test]
fn a_turn_token_leads_before_the_lexical_block() {
    // With both a token and a salient lexical hit, the token line leads and the "possibly relevant"
    // block follows.
    let bonsai = MemoryId::generate();
    let graph = corpus(topic(
        bonsai,
        "bonsai",
        "A schema-migration tool Erin built; it versions and applies database migrations.",
    ));
    let turn = TurnId::generate();
    let message = format!(
        "What do you think of bonsai, given {}?",
        turn_ref::construct(turn)
    );
    let hint = ambient_recall(
        &graph,
        &AmbientSettings::default(),
        &message,
        &HashSet::new(),
        true,
        true,
    )
    .unwrap()
    .expect("both a token and a lexical hit surface");
    assert_eq!(hint.hits.len(), 1);
    let token_line = hint
        .message
        .lines()
        .position(|l| l.contains("convo.turn"))
        .expect("a token line");
    let lexical_line = hint
        .message
        .lines()
        .position(|l| l.contains("Possibly relevant"))
        .expect("the lexical block");
    assert!(
        token_line < lexical_line,
        "the token line leads: {}",
        hint.message
    );
}

#[test]
fn a_turn_token_is_silent_when_transcripts_are_off() {
    // The convo.turn resolver is transcripts-gated, so with the feature off a token yields no line —
    // and, with no lexical hit either, no hint at all (nudging at a nil call would be cruel).
    let bonsai = MemoryId::generate();
    let graph = corpus(topic(
        bonsai,
        "bonsai",
        "A schema-migration tool Erin built.",
    ));
    let turn = TurnId::generate();
    let message = format!(
        "Can you dig up what we said in {}?",
        turn_ref::construct(turn)
    );
    let hint = ambient_recall(
        &graph,
        &AmbientSettings::default(),
        &message,
        &HashSet::new(),
        false,
        true,
    )
    .unwrap();
    assert!(
        hint.is_none(),
        "no token line and no lexical hit means no hint"
    );
}

#[test]
fn the_token_lead_caps_at_the_first_few() {
    // A message citing many moments names only the first MAX_TURN_TOKENS, so the lead-in stays terse.
    let turns: Vec<TurnId> = (0..5).map(|_| TurnId::generate()).collect();
    let mut message = String::from("Compare these:");
    for turn in &turns {
        message.push(' ');
        message.push_str(&turn_ref::construct(*turn));
    }
    let graph = corpus(Vec::new());
    let hint = ambient_recall(
        &graph,
        &AmbientSettings::default(),
        &message,
        &HashSet::new(),
        true,
        true,
    )
    .unwrap()
    .expect("the tokens fire the hint");
    let token_lines = hint
        .message
        .lines()
        .filter(|l| l.contains("convo.turn"))
        .count();
    assert_eq!(token_lines, MAX_TURN_TOKENS, "the lead-in is capped");
}
