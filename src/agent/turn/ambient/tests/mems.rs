//! Tests for the memory-reference lead: a spliced `[mem:<id>]` decodes to its handle and leads the
//! hint, collapses a merged member to the class primary, stays silent when unresolvable, caps at the
//! first few, and orders ahead of the turn and URL lines.

use std::collections::HashSet;

use super::{corpus, merged_rowan, person};
use crate::{
    agent::turn::ambient::{MAX_MEM_TOKENS, ambient_recall},
    ids::{MemoryId, TurnId},
    mem_ref,
    settings::AmbientSettings,
    turn_ref,
};

#[test]
fn a_mem_reference_fires_the_hint_without_a_lexical_hit() {
    // A message carrying a spliced `[mem:<id>]` but matching nothing lexically still surfaces a
    // hint: the mem line decodes the token to its handle, so the reference is never opaque.
    let rowan = MemoryId::generate();
    let graph = corpus(person(rowan, "person/rowan@chat", "Runs the boat crew."));
    let message = format!("is {} reachable right now?", mem_ref::construct(rowan));
    let hint = ambient_recall(
        &graph,
        &AmbientSettings::default(),
        &message,
        &HashSet::new(),
        true,
        true,
    )
    .unwrap()
    .expect("the mem token fires the hint even with no lexical hit");
    assert!(hint.hits.is_empty(), "no lexical hit rode along");
    let first = hint.message.lines().next().unwrap();
    assert!(
        first.contains(&format!("[mem:{}] refers to person/rowan@chat", rowan.0)),
        "the hint leads with the decoded token: {first}"
    );
}

#[test]
fn a_referenced_merged_member_names_the_class_primary() {
    // Referencing the non-primary member's token collapses to the class primary's handle, so a
    // merged identity reads under one handle — but the line still names the token as the message
    // wrote it.
    let (graph, direct, chat) = merged_rowan();
    let primary = direct.min(chat);
    let non_primary = direct.max(chat);
    let message = format!("has {} been around?", mem_ref::construct(non_primary));
    let hint = ambient_recall(
        &graph,
        &AmbientSettings::default(),
        &message,
        &HashSet::new(),
        true,
        true,
    )
    .unwrap()
    .expect("the mem token fires the hint");
    let primary_name = graph.memory_by_id(primary).unwrap().unwrap().name;
    let mem_line = hint
        .message
        .lines()
        .find(|l| l.contains("refers to"))
        .expect("a mem line");
    assert!(
        mem_line.contains(primary_name.as_str()),
        "the line names the class primary: {mem_line}"
    );
    assert!(
        mem_line.contains(&format!("[mem:{}]", non_primary.0)),
        "the line names the token as written: {mem_line}"
    );
}

#[test]
fn an_unresolvable_mem_reference_is_silent() {
    // A token for a memory that does not exist here (perhaps from another instance) resolves to no
    // handle, so it gets no line — and with no lexical hit either, no hint at all.
    let graph = corpus(Vec::new());
    let ghost = MemoryId::generate();
    let message = format!("what about {}?", mem_ref::construct(ghost));
    let hint = ambient_recall(
        &graph,
        &AmbientSettings::default(),
        &message,
        &HashSet::new(),
        true,
        true,
    )
    .unwrap();
    assert!(
        hint.is_none(),
        "an unresolvable mem reference yields no line and no hint"
    );
}

#[test]
fn the_mem_lead_caps_at_the_first_few() {
    // A message citing many mem references names only the first MAX_MEM_TOKENS, so the lead-in
    // stays terse.
    let ids: Vec<MemoryId> = (0..5).map(|_| MemoryId::generate()).collect();
    let mut payloads = Vec::new();
    for (i, id) in ids.iter().enumerate() {
        payloads.extend(person(
            *id,
            &format!("person/user{i}@chat"),
            "a crew member",
        ));
    }
    let graph = corpus(payloads);
    let mut message = String::from("who among these:");
    for id in &ids {
        message.push(' ');
        message.push_str(&mem_ref::construct(*id));
    }
    let hint = ambient_recall(
        &graph,
        &AmbientSettings::default(),
        &message,
        &HashSet::new(),
        true,
        true,
    )
    .unwrap()
    .expect("the mem tokens fire the hint");
    let mem_lines = hint
        .message
        .lines()
        .filter(|l| l.contains("refers to"))
        .count();
    assert_eq!(mem_lines, MAX_MEM_TOKENS, "the mem lead-in is capped");
}

#[test]
fn a_mem_line_leads_before_the_turn_and_url_lines() {
    // With a mem reference, a turn token, and a URL, the order is: mem line, then turn line, then
    // URL line.
    let rowan = MemoryId::generate();
    let graph = corpus(person(rowan, "person/rowan@chat", "Runs the boat crew."));
    let turn = TurnId::generate();
    let message = format!(
        "has {} seen {} at https://example.com/x?",
        mem_ref::construct(rowan),
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
    .expect("a mem ref, a turn token, and a URL surface");
    let mem_line = hint
        .message
        .lines()
        .position(|l| l.contains("refers to"))
        .expect("a mem line");
    let token_line = hint
        .message
        .lines()
        .position(|l| l.contains("convo.turn"))
        .expect("a token line");
    let url_line = hint
        .message
        .lines()
        .position(|l| l.contains("web.markdown"))
        .expect("a URL line");
    assert!(
        mem_line < token_line && token_line < url_line,
        "mem, then turn, then URL: {}",
        hint.message
    );
}
