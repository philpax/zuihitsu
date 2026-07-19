//! The turn-reference vocabulary — the constructor that mints the syntax letting the agent and a
//! frontend point back to a specific conversation moment (spec §Conversations → Transcripts).
//!
//! A reference names a `ConversationTurn` by its 26-character Crockford [`TurnId`], carried in the
//! canonical `[turn:<ulid>]` token. [`construct`] mints that token; the parser that finds, normalizes,
//! and extracts references — over both this vocabulary and [`crate::mem_ref`] — is [`crate::message_refs`].
//!
//! # Tokens, not URLs
//!
//! The token is the only form the agent's side of the boundary knows: the resolver (`convo.turn(id)`
//! in `src/agent/lua`) reads a bare ULID that it only ever sees inside a token. A deep-link URL that
//! points at the same moment is each frontend's own concern — recognizing one is route matching against
//! that frontend's URL grammar, and a connector rewrites such a link to this token before the message
//! reaches the agent. So this module carries no URL awareness; it is the canonical, agent-facing token
//! vocabulary alone.

use crate::{ids::TurnId, message_refs::TURN_OPEN};

/// The canonical reference token for a turn: `[turn:<ulid>]`. What the agent copies to cite a moment,
/// and the form every turn reference collapses to on [`crate::message_refs::normalize`].
pub fn construct(turn: TurnId) -> String {
    format!("{TURN_OPEN}{}]", turn.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message_refs::{self, Segment, ULID_LEN};
    use ulid::Ulid;

    fn turn_id(bits: u128) -> TurnId {
        TurnId(Ulid::from(bits))
    }

    #[test]
    fn construct_is_the_canonical_bracket_token() {
        let turn = turn_id(1);
        let token = construct(turn);
        assert!(token.starts_with("[turn:"));
        assert!(token.ends_with(']'));
        assert_eq!(token.len(), "[turn:]".len() + ULID_LEN);
        assert_eq!(message_refs::scan(&token), vec![Segment::Turn(turn)]);
    }
}
