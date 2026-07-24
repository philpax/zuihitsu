use super::*;
use crate::{
    TEST_PLATFORM,
    event::TurnRole,
    ids::{Seq, TurnId},
    time::Timestamp,
};

fn turn(seq: u64, text: &str) -> TurnView {
    TurnView {
        seq: Seq(seq),
        turn_id: TurnId::generate(),
        role: TurnRole::Participant,
        text: text.to_owned(),
        participant: None,
        recorded_at: Timestamp::from_millis(0),
        steps: Vec::new(),
        produced_by: None,
    }
}

#[test]
fn estimate_tokens_counts_buffer_and_messages() {
    let buffer = vec![turn(1, "12345678")]; // 8 chars
    // (8 + 4) / 4 = 3.
    let messages = vec![MessageInput {
        sender: PersonId::new(TEST_PLATFORM, "dave"),
        text: "1234".to_owned(),
    }];
    assert_eq!(estimate_tokens(&buffer, &messages), 3);
}
