//! The memory-reference vocabulary — the constructor that mints the syntax letting the agent, a
//! connector, and a frontend point at a specific memory (a person, an event, a place).
//!
//! A reference names a memory by its immutable 26-character Crockford [`MemoryId`], carried in the
//! canonical `[mem:<ulid>]` token. [`construct`] mints that token; the parser that finds, normalizes,
//! and extracts references — over both this vocabulary and [`crate::turn_ref`] — is
//! [`crate::message_refs`].
//!
//! # Tokens, not URLs
//!
//! Like [`crate::turn_ref`], this module carries no URL awareness. A memory's deep-link URL routes by
//! *handle*, not by id, so recognizing one means matching a frontend's route and resolving a handle to a
//! [`MemoryId`] — that frontend's own concern (route knowledge plus a graph query), not this
//! dependency-light core module's. So this is the canonical, agent-facing token vocabulary alone; a
//! frontend maps its own URLs to these tokens.

use crate::{ids::MemoryId, message_refs::MEM_OPEN};

/// The canonical reference token for a memory: `[mem:<ulid>]`. What a connector splices in to point at a
/// memory, and the form every memory reference collapses to on [`crate::message_refs::normalize`].
pub fn construct(memory: MemoryId) -> String {
    format!("{MEM_OPEN}{}]", memory.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message_refs::{self, Segment, ULID_LEN};
    use ulid::Ulid;

    fn memory_id(bits: u128) -> MemoryId {
        MemoryId(Ulid::from(bits))
    }

    #[test]
    fn construct_is_the_canonical_bracket_token() {
        let memory = memory_id(1);
        let token = construct(memory);
        assert!(token.starts_with("[mem:"));
        assert!(token.ends_with(']'));
        assert_eq!(token.len(), "[mem:]".len() + ULID_LEN);
        assert_eq!(message_refs::scan(&token), vec![Segment::Mem(memory)]);
    }
}
