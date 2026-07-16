//! Session reads: conversation resolution, participants, and open-session state.

use crate::{
    db::{query_map_into, query_opt_into},
    event::{ConversationRef, Teller},
    graph::{
        Graph, GraphError, OpenSessionView, ParticipantMint, SessionView, backend, parse_ulid,
    },
    ids::{ConversationId, ConversationLocator, MemoryId, MemoryName, Namespace, Seq, SessionId},
    time::Timestamp,
    visibility::{MarkerRoom, MarkerTurn, room_display},
    vocabulary::TagName,
};
use rusqlite::{OptionalExtension, params};

impl Graph {
    /// Resolve a conversation's locator to its id, or `None` if the room has never been seen. A
    /// retired (ended) conversation still resolves — the room is durable.
    pub fn conversation_for_locator(
        &self,
        locator: &ConversationLocator,
    ) -> Result<Option<ConversationId>, GraphError> {
        let id: Option<String> = self
            .conn
            .query_row(
                "SELECT id FROM conversations WHERE platform = ?1 AND scope_path = ?2",
                params![locator.platform.as_str(), locator.scope_path.as_str()],
                |r| r.get(0),
            )
            .optional()
            .map_err(backend)?;
        id.map(|id| parse_ulid(&id).map(ConversationId)).transpose()
    }

    /// Resolve a platform participant `(platform, platform_user_id)` to its [`Namespace::Person`]
    /// stub, or `None` if that identity has never been seen.
    pub fn participant_for(
        &self,
        platform: &str,
        platform_user_id: &str,
    ) -> Result<Option<MemoryId>, GraphError> {
        let id: Option<String> = self
            .conn
            .query_row(
                "SELECT memory FROM participant_identities
                 WHERE platform = ?1 AND platform_user_id = ?2",
                params![platform, platform_user_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(backend)?;
        id.map(|id| parse_ulid(&id).map(MemoryId)).transpose()
    }

    /// The memory name a freshly minted [`Namespace::Person`] participant would receive, given
    /// their platform handle and the platform they arrived on. The name half of
    /// [`Graph::participant_mint`]: shared by
    /// the console's optimistic preview, so the name the console shows before the event lands is the
    /// same name the server will assign.
    pub fn participant_name(
        &self,
        platform: &str,
        platform_user_id: &str,
    ) -> Result<MemoryName, GraphError> {
        Ok(self.participant_mint(platform, platform_user_id)?.name)
    }

    /// The plan for minting a fresh [`Namespace::Person`] stub for `(platform, platform_user_id)`:
    /// the qualified name `person/<platform_user_id>@<platform>`. The caller
    /// (`resolve_or_mint_participant`) checks whether the qualified name already exists as a
    /// memory (an agent-authored hearsay stub) and binds the platform identity to it, or creates a
    /// fresh memory.
    pub fn participant_mint(
        &self,
        platform: &str,
        platform_user_id: &str,
    ) -> Result<ParticipantMint, GraphError> {
        let name: MemoryName = Namespace::Person
            .with_name(format!("{platform_user_id}@{platform}"))
            .into();
        Ok(ParticipantMint { name })
    }

    /// The platform user ids seen on a given platform — the bare handles a user can type in the
    /// "you are" field, sourced from the `participant_identities` table rather than memory subjects,
    /// so the `@platform` disambiguation suffix never surfaces as a separate entry.
    pub fn participant_ids_for(&self, platform: &str) -> Result<Vec<String>, GraphError> {
        let stmt = self.conn.prepare(
            "SELECT DISTINCT platform_user_id FROM participant_identities
             WHERE platform = ?1 ORDER BY platform_user_id",
        )?;
        Ok(query_map_into(stmt, params![platform], |row| row.get(0))?)
    }

    /// The [`Namespace::Context`] memory minted with a conversation, or `None` if the conversation
    /// is unknown. The locator resolves to the room and thence to its context (spec §Contexts are
    /// first-class).
    pub fn context_for_conversation(
        &self,
        conversation: ConversationId,
    ) -> Result<Option<MemoryId>, GraphError> {
        let id: Option<String> = self
            .conn
            .query_row(
                "SELECT context_memory FROM conversations WHERE id = ?1",
                params![conversation.0.to_string()],
                |r| r.get(0),
            )
            .optional()
            .map_err(backend)?;
        id.map(|id| parse_ulid(&id).map(MemoryId)).transpose()
    }

    /// Resolve a teller to the display name a marker shows (a participant's handle, or a fixed label
    /// for the agent and genesis). Shared by search and brief composition.
    pub fn teller_display(&self, teller: &Teller) -> Result<String, GraphError> {
        Ok(match teller {
            Teller::Participant(id) => self
                .memory_by_id(*id)?
                .map(|memory| memory.name.as_str().to_owned())
                .unwrap_or_else(|| "someone".to_owned()),
            Teller::Agent => "the agent".to_owned(),
            Teller::Bootstrap => "genesis".to_owned(),
        })
    }

    /// Resolve a `told_in` conversation reference to its marker — the turn id (for cross-linking)
    /// and the resolved room (display name + `#confidential` flag) — for the visibility marker.
    /// `None` when the entry carries no reference. Shared by search and brief composition, both of
    /// which bake the marker at build time (spec §Visibility → marker). When the reference's turn
    /// is `Some`, the turn id is carried for the `[turn:<ulid>]` token; the room is resolved from
    /// the conversation's context memory regardless.
    pub fn marker_ref(&self, told_in: Option<&ConversationRef>) -> Result<MarkerTurn, GraphError> {
        let Some(r) = told_in else {
            return Ok(MarkerTurn {
                turn_id: None,
                room: None,
            });
        };
        let context_memory = self.context_for_conversation(r.conversation)?;
        let room = context_memory.and_then(|context_id| {
            self.memory_by_id(context_id)
                .ok()
                .flatten()
                .map(|context| MarkerRoom {
                    name: room_display(context.name.as_str()),
                    confidential: context.tags.contains(&TagName::Confidential),
                })
        });
        Ok(MarkerTurn {
            turn_id: r.turn,
            room,
        })
    }

    /// A session by id, with its participants, or `None` if unknown.
    pub fn session(&self, id: SessionId) -> Result<Option<SessionView>, GraphError> {
        let stmt = self.session_stmt("WHERE id = ?1")?;
        query_opt_into(stmt, params![id.0.to_string()], |row| {
            self.assemble_session(row)
        })
    }

    /// A conversation's sessions, oldest first (commit order).
    pub fn sessions_in(
        &self,
        conversation: ConversationId,
    ) -> Result<Vec<SessionView>, GraphError> {
        let stmt = self.session_stmt("WHERE conversation = ?1 ORDER BY seq")?;
        query_map_into(stmt, params![conversation.0.to_string()], |row| {
            self.assemble_session(row)
        })
    }

    /// The most recent unclosed session of a conversation — the live one a restart must recover — or
    /// `None` if every session has ended. The in-memory session map is process-local, so on boot this
    /// is how a session still open in the log (left by a restart, or a passive graceful exit) is found
    /// again, to resume within the idle gap or close-with-flush past it.
    pub fn last_open_session(
        &self,
        conversation: ConversationId,
    ) -> Result<Option<OpenSessionView>, GraphError> {
        let stmt = self.conn.prepare(
            "SELECT id, brief, started_at, seq, seeded_from_turn FROM sessions
             WHERE conversation = ?1 AND ended = 0 ORDER BY seq DESC LIMIT 1",
        )?;
        query_opt_into(stmt, params![conversation.0.to_string()], |row| {
            let id: String = row.get("id")?;
            let seeded: Option<String> = row.get("seeded_from_turn")?;
            Ok::<_, GraphError>(OpenSessionView {
                id: SessionId(parse_ulid(&id)?),
                brief: row.get("brief")?,
                started_at: Timestamp::from_millis(row.get("started_at")?),
                start_seq: Seq(row.get::<_, i64>("seq")? as u64),
                seeded: seeded.is_some(),
            })
        })
    }

    /// Whether `conversation` has a session that opened before `session` — i.e. `session` is not its
    /// first. The operator imprint reads this to leave imprint mode once onboarding is done (spec
    /// §Imprint interview): the first operator session runs the imprint template, and every session
    /// after it the ordinary scaffold, so the channel stops re-running the create-a-profile script.
    pub fn has_earlier_session(
        &self,
        conversation: ConversationId,
        session: SessionId,
    ) -> Result<bool, GraphError> {
        self.conn
            .query_row(
                "SELECT EXISTS(
                     SELECT 1 FROM sessions
                     WHERE conversation = ?1
                       AND seq < (SELECT seq FROM sessions WHERE id = ?2)
                 )",
                params![conversation.0.to_string(), session.0.to_string()],
                |row| row.get(0),
            )
            .map_err(backend)
    }

    /// Every conversation's current open session (its latest un-ended one), paired with the
    /// conversation — the idle sweep's input, so it can close-with-flush the stale ones. A conversation
    /// has one live session, but a pre-fix log may also hold earlier dangling ones (a crash that never
    /// recorded `SessionEnded`); only the latest per conversation is returned, so the sweep never
    /// reopens a zombie.
    pub fn open_sessions(&self) -> Result<Vec<(ConversationId, OpenSessionView)>, GraphError> {
        let stmt = self.conn.prepare(
            "SELECT conversation, id, brief, started_at, seq, seeded_from_turn FROM sessions s
             WHERE ended = 0
               AND seq = (SELECT MAX(seq) FROM sessions
                          WHERE conversation = s.conversation AND ended = 0)
             ORDER BY seq",
        )?;
        query_map_into(stmt, [], |row| {
            let conversation: String = row.get("conversation")?;
            let id: String = row.get("id")?;
            let seeded: Option<String> = row.get("seeded_from_turn")?;
            Ok::<_, GraphError>((
                ConversationId(parse_ulid(&conversation)?),
                OpenSessionView {
                    id: SessionId(parse_ulid(&id)?),
                    brief: row.get("brief")?,
                    started_at: Timestamp::from_millis(row.get("started_at")?),
                    start_seq: Seq(row.get::<_, i64>("seq")? as u64),
                    seeded: seeded.is_some(),
                },
            ))
        })
    }

    /// Prepare a `sessions` read over the columns [`Graph::assemble_session`] decodes, with `clause`
    /// supplying the differing `WHERE` (and any `ORDER BY`). Sharing the column list keeps the by-id
    /// and by-conversation reads provably returning the same row shape. `clause` is a static fragment,
    /// never agent input.
    fn session_stmt(&self, clause: &str) -> Result<rusqlite::Statement<'_>, GraphError> {
        Ok(self.conn.prepare(&format!(
            "SELECT id, conversation, started_at, seeded_from_turn, brief FROM sessions {clause}"
        ))?)
    }

    /// Whether a session is still open — recorded but not yet `SessionEnded`. The close paths check this
    /// before flushing and ending, so a session the idle sweep snapshotted as open (its candidate list is
    /// captured up front) but another path has since closed is not flushed and ended a second time.
    pub fn session_is_open(&self, session: SessionId) -> Result<bool, GraphError> {
        Ok(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE id = ?1 AND ended = 0)",
            params![session.0.to_string()],
            |row| row.get(0),
        )?)
    }

    /// A session's participants — the present set at open plus anyone who joined — ordered by id.
    pub fn session_participants(&self, session: SessionId) -> Result<Vec<MemoryId>, GraphError> {
        let stmt = self.conn.prepare(
            "SELECT memory FROM session_participants WHERE session = ?1 ORDER BY memory",
        )?;
        query_map_into(stmt, params![session.0.to_string()], |row| {
            let memory: String = row.get(0)?;
            Ok(MemoryId(parse_ulid(&memory)?))
        })
    }

    /// Build a [`SessionView`] from a row selecting the columns [`Graph::session_stmt`] lists, then
    /// load its participants. Decoding from the row here keeps the column list and its reader together.
    fn assemble_session(&self, row: &rusqlite::Row<'_>) -> Result<SessionView, GraphError> {
        let id: String = row.get("id")?;
        let conversation: String = row.get("conversation")?;
        let seeded_from_turn: Option<String> = row.get("seeded_from_turn")?;
        let id = SessionId(parse_ulid(&id)?);
        Ok(SessionView {
            id,
            conversation: ConversationId(parse_ulid(&conversation)?),
            started_at: Timestamp::from_millis(row.get("started_at")?),
            seeded_from_turn: seeded_from_turn
                .map(|json| serde_json::from_str(&json))
                .transpose()?,
            brief: row.get("brief")?,
            participants: self.session_participants(id)?,
        })
    }
}
