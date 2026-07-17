//! Session and conversation event materialization arms, extracted from
//! [`Graph::apply`](crate::graph::Graph::apply).

use rusqlite::params;

use crate::{
    event::{Event, EventPayload},
    graph::{GraphError, backend},
};

use crate::graph::Graph;

impl Graph {
    /// Materialize the session/conversation-event arm of
    /// [`Graph::apply`](crate::graph::Graph::apply). Returns `Ok(true)` if the payload was a session
    /// event and was handled, `Ok(false)` otherwise.
    pub(super) fn apply_session_event(&mut self, event: &Event) -> Result<bool, GraphError> {
        match &event.payload {
            EventPayload::ConversationStarted {
                id,
                locator,
                context_memory,
            } => {
                // Idempotent: the room is opened once; a re-seen locator is a no-op, not a duplicate.
                self.conn
                    .execute(
                        "INSERT OR IGNORE INTO conversations (id, platform, scope_path, context_memory)
                         VALUES (?1, ?2, ?3, ?4)",
                        params![
                            id.0.to_string(),
                            locator.platform.as_str(),
                            locator.scope_path.as_str(),
                            context_memory.0.to_string(),
                        ],
                    )
                    .map_err(backend)?;
            }
            EventPayload::ConversationEnded { id } => {
                self.conn
                    .execute(
                        "UPDATE conversations SET ended = 1 WHERE id = ?1",
                        params![id.0.to_string()],
                    )
                    .map_err(backend)?;
            }
            EventPayload::SessionStarted {
                conversation,
                id,
                participants,
                started_at,
                seeded_from_turn,
                brief,
                // The working set and initiators are replay metadata for after-the-fact brief
                // re-derivation; the materialized graph does not consume them.
                working_set: _,
                initiators: _,
            } => {
                self.conn
                    .execute(
                        "INSERT INTO sessions
                         (id, conversation, started_at, seeded_from_turn, brief, seq)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        params![
                            id.0.to_string(),
                            conversation.0.to_string(),
                            started_at.as_millis(),
                            seeded_from_turn
                                .as_ref()
                                .map(|r| {
                                    serde_json::to_string(r).map_err(GraphError::Serialize)
                                })
                                .transpose()?,
                            brief,
                            event.seq.0 as i64,
                        ],
                    )
                    .map_err(backend)?;
                // The present set at open carries no joining turn; a join records its `at_turn`.
                for participant in participants {
                    self.conn
                        .execute(
                            "INSERT OR IGNORE INTO session_participants (session, memory, at_turn)
                             VALUES (?1, ?2, NULL)",
                            params![id.0.to_string(), participant.0.to_string()],
                        )
                        .map_err(backend)?;
                }
            }
            EventPayload::SessionEnded { id, cause, .. } => {
                // Record the close cause alongside the `ended` flag — provenance for the console and
                // analytics. `None` (a pre-cause log) leaves the column NULL.
                self.conn
                    .execute(
                        "UPDATE sessions SET ended = 1, end_cause = ?2 WHERE id = ?1",
                        params![
                            id.0.to_string(),
                            cause
                                .as_ref()
                                .map(|c| serde_json::to_string(c).map_err(GraphError::Serialize))
                                .transpose()?,
                        ],
                    )
                    .map_err(backend)?;
            }
            EventPayload::ParticipantJoined {
                session,
                participant,
                at_turn,
                ..
            } => {
                self.conn
                    .execute(
                        "INSERT OR IGNORE INTO session_participants (session, memory, at_turn)
                         VALUES (?1, ?2, ?3)",
                        params![
                            session.0.to_string(),
                            participant.0.to_string(),
                            serde_json::to_string(at_turn).map_err(GraphError::Serialize)?,
                        ],
                    )
                    .map_err(backend)?;
            }
            EventPayload::ParticipantIdentified {
                memory,
                platform,
                platform_user_id,
            } => {
                self.conn
                    .execute(
                        "INSERT OR IGNORE INTO participant_identities
                         (platform, platform_user_id, memory) VALUES (?1, ?2, ?3)",
                        params![
                            platform.as_str(),
                            platform_user_id.as_str(),
                            memory.0.to_string(),
                        ],
                    )
                    .map_err(backend)?;
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
}
