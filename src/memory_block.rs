//! The block transaction: the agent's memory-mutation surface as one opaque, invariant-enforcing
//! object.
//!
//! A [`MemoryBlock`] accumulates the side-effect events of a single Lua block — creates, appends,
//! links — committed or discarded atomically by the caller. It owns the buffer and the touched set,
//! resolves reads against the graph overlaid with its own pending writes (read-your-writes), and is
//! the one place the write invariants live: name uniqueness, registered relations, and the
//! write-time visibility default (including the `#confidential`-room firming). The Lua layer
//! ([`crate::lua`]) is a thin wrapper over this — it translates script calls into method calls and
//! never touches the buffer, the events, or the visibility rules directly.

use std::collections::BTreeSet;

use serde::Deserialize;

use crate::{
    clock::Clock,
    event::{EventPayload, LinkSource, Teller, Visibility},
    graph::{Graph, GraphError},
    ids::{ConversationId, EntryId, MemoryId, MemoryName, RelationName, TagName},
    visibility::default_visibility_named,
};

/// Who is driving a block's writes. Operator authority is the control panel; it is the only path
/// permitted to edit `self`, and it authors its links as `Debugger` rather than `Agent` (spec
/// §Imprint interview). Platform authority is an ordinary conversation turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Authority {
    Platform,
    Operator,
}

/// One block's in-progress memory mutations. Built fresh per block, mutated through its operations,
/// and consumed by [`MemoryBlock::into_effects`] to commit or discard.
pub struct MemoryBlock<'a> {
    graph: &'a Graph,
    clock: &'a dyn Clock,
    /// The turn's teller, attributed to content written this block unless an append opts out.
    teller: Teller,
    /// Whether this block runs under platform or operator authority — gates `self` writes and the
    /// link source.
    authority: Authority,
    /// The `self` memory's id, resolved once at open so every write path can guard it with a cheap id
    /// compare. `None` only before genesis seeds `self`.
    self_id: Option<MemoryId>,
    /// The current conversation's `context/*` memory (where content is told in), if any.
    told_in: Option<MemoryId>,
    /// Whether `told_in` carries the `#confidential` tag — content here defaults private.
    confidential_context: bool,
    buffer: Vec<EventPayload>,
    touched: BTreeSet<MemoryId>,
    aborted: Option<String>,
}

/// What a finished block yields to its caller for commit (or, on abort/error, to discard): the
/// buffered side effects, the memories touched (the lock set, for compaction's working-set), and the
/// abort reason if [`MemoryBlock::abort`] was called.
pub struct BlockEffects {
    pub events: Vec<EventPayload>,
    pub touched: Vec<MemoryId>,
    pub aborted: Option<String>,
}

/// A write that violates an invariant, surfaced to the agent as a teachable error, or an underlying
/// graph read failure. `Display` is the agent-facing message (the Lua layer renders it as the
/// block's terminal cause), so it is deliberately unprefixed — the agent reads it, not an operator.
#[derive(Debug)]
pub enum MemoryError {
    /// A `create` collided with an existing name (names are unique).
    NameExists(MemoryName),
    /// A `link`/`unlink` named a relation that is not a registered link type.
    UnknownRelation(RelationName),
    /// A platform-authority write tried to touch `self` — appending to it, or linking from or to it.
    /// Only the control panel (operator authority) may edit `self`.
    SelfWriteForbidden,
    /// A platform-authority write tried to assert or retract a `same_as` merge. Cross-platform
    /// identity is operator-asserted only — the agent never merges two identities on its own.
    MergeForbidden,
    /// A graph read failed — infrastructure, not the agent's doing.
    Graph(GraphError),
}

impl std::fmt::Display for MemoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemoryError::NameExists(name) => write!(
                f,
                "a memory named {:?} already exists; fetch it with memory.get",
                name.as_str()
            ),
            MemoryError::UnknownRelation(relation) => write!(
                f,
                "unknown relation {:?}; it must be a registered link type",
                relation.as_str()
            ),
            MemoryError::SelfWriteForbidden => {
                write!(f, "self can only be edited from the control panel")
            }
            MemoryError::MergeForbidden => {
                write!(
                    f,
                    "same_as merges can only be asserted from the control panel"
                )
            }
            MemoryError::Graph(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for MemoryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            MemoryError::Graph(error) => Some(error),
            _ => None,
        }
    }
}

impl From<GraphError> for MemoryError {
    fn from(error: GraphError) -> MemoryError {
        MemoryError::Graph(error)
    }
}

/// The forced visibility a `visibility = "public" | "private"` append opt selects, deserialized from
/// the Lua opts table.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VisibilityChoice {
    Public,
    Private,
}

/// The overrides an append accepts: `by_agent` records it as the agent's own observation rather than
/// the speaker's; `visibility` forces the visibility instead of the write-time default. Deserialized
/// straight from the Lua `opts` table.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct AppendOptions {
    pub by_agent: bool,
    pub visibility: Option<VisibilityChoice>,
}

impl<'a> MemoryBlock<'a> {
    /// Open a block for `conversation`: resolve the context it writes in and whether that room is
    /// `#confidential`. Fails only on a graph read error (infrastructure), never on agent input.
    pub fn new(
        graph: &'a Graph,
        clock: &'a dyn Clock,
        teller: Teller,
        authority: Authority,
        conversation: ConversationId,
    ) -> Result<MemoryBlock<'a>, GraphError> {
        let told_in = graph.context_for_conversation(conversation)?;
        let confidential_context = match told_in {
            Some(context_id) => graph
                .memory_by_id(context_id)?
                .is_some_and(|context| context.tags.contains(&TagName::Confidential)),
            None => false,
        };
        let self_id = graph
            .memory_by_name(MemoryName::SELF)?
            .map(|memory| memory.id);
        Ok(MemoryBlock {
            graph,
            clock,
            teller,
            authority,
            self_id,
            told_in,
            confidential_context,
            buffer: Vec::new(),
            touched: BTreeSet::new(),
            aborted: None,
        })
    }

    /// Create a memory, optionally with a first content entry. The name must be free — a collision is
    /// a teachable error rejected before anything is buffered, so a duplicate `MemoryCreated` never
    /// reaches the log (where it would only fail at materialize, poisoning replay).
    pub fn create(&mut self, name: &str, content: Option<&str>) -> Result<MemoryId, MemoryError> {
        if self.resolve(name)?.is_some() {
            return Err(MemoryError::NameExists(MemoryName::new(name)));
        }
        let id = MemoryId::generate();
        self.touched.insert(id);
        self.buffer.push(EventPayload::MemoryCreated {
            id,
            name: MemoryName::new(name),
        });
        if let Some(text) = content {
            // A first entry is told like any append: by the turn's teller, at the write-time default
            // visibility for the new memory's name.
            let teller = self.teller.clone();
            let visibility = self.default_visibility(name, id, &teller);
            self.push_content(id, text.to_owned(), teller, visibility);
        }
        Ok(id)
    }

    /// Resolve a name to a memory id, or `None`, for `memory.get` — touches the result so it enters
    /// the lock set.
    pub fn get(&mut self, name: &str) -> Result<Option<MemoryId>, MemoryError> {
        let resolved = self.resolve(name)?;
        if let Some(id) = resolved {
            self.touched.insert(id);
        }
        Ok(resolved)
    }

    /// Append a content entry to `id`. `opts.by_agent` attributes it to the agent; `opts.visibility`
    /// forces the visibility; otherwise the write-time default applies (a `#confidential` room, or an
    /// aside about an absent third party, defaults private to the teller).
    pub fn append(
        &mut self,
        id: MemoryId,
        text: &str,
        opts: AppendOptions,
    ) -> Result<(), MemoryError> {
        self.guard_self(id)?;
        let told_by = if opts.by_agent {
            Teller::Agent
        } else {
            self.teller.clone()
        };
        let visibility = match opts.visibility {
            Some(VisibilityChoice::Public) => Visibility::Public,
            Some(VisibilityChoice::Private) => Visibility::PrivateToTeller,
            None => match self.resolve_name(id)? {
                Some(name) => self.default_visibility(name.as_str(), id, &told_by),
                None if self.confidential_context => Visibility::PrivateToTeller,
                None => Visibility::Public,
            },
        };
        self.push_content(id, text.to_owned(), told_by, visibility);
        Ok(())
    }

    /// The memory's content entry texts: its whole `same_as` class from the graph plus this block's
    /// pending appends. A traversing read, so it touches every class member, not just `id`.
    pub fn entries(&mut self, id: MemoryId) -> Result<Vec<String>, MemoryError> {
        let members = self.graph.class_members(id)?;
        self.touched.insert(id);
        for member in &members {
            self.touched.insert(*member);
        }
        let mut texts: Vec<String> = self
            .graph
            .class_entries(id)?
            .into_iter()
            .map(|entry| entry.text)
            .collect();
        for event in &self.buffer {
            if let EventPayload::MemoryContentAppended {
                id: entry_id, text, ..
            } = event
                && *entry_id == id
            {
                texts.push(text.clone());
            }
        }
        Ok(texts)
    }

    /// Link `from` to `to` under a registered relation (e.g. flag a thread `active_in` the context).
    pub fn link(
        &mut self,
        from: MemoryId,
        to: MemoryId,
        relation: RelationName,
    ) -> Result<(), MemoryError> {
        self.change_link(from, to, relation, true)
    }

    /// Remove such a link.
    pub fn unlink(
        &mut self,
        from: MemoryId,
        to: MemoryId,
        relation: RelationName,
    ) -> Result<(), MemoryError> {
        self.change_link(from, to, relation, false)
    }

    /// The current conversation's context memory, or `None` — touches it so it enters the lock set.
    pub fn current_context(&mut self) -> Option<MemoryId> {
        if let Some(id) = self.told_in {
            self.touched.insert(id);
        }
        self.told_in
    }

    /// Discard everything this block buffered and end it, recording `reason` as the terminal cause.
    pub fn abort(&mut self, reason: Option<String>) {
        self.aborted = Some(reason.unwrap_or_default());
    }

    /// Consume the block for commit: the buffered events, the touched lock set, and any abort reason.
    pub fn into_effects(self) -> BlockEffects {
        BlockEffects {
            events: self.buffer,
            touched: self.touched.into_iter().collect(),
            aborted: self.aborted,
        }
    }

    /// Enforce that `relation` is registered — the graph stores an unregistered relation as given, so
    /// the contract is checked here — then buffer the create/remove and touch both endpoints.
    fn change_link(
        &mut self,
        from: MemoryId,
        to: MemoryId,
        relation: RelationName,
        create: bool,
    ) -> Result<(), MemoryError> {
        if self.graph.relation(relation.as_str())?.is_none() {
            return Err(MemoryError::UnknownRelation(relation));
        }
        // Cross-platform identity is operator-asserted only: a participant must not be able to steer
        // the agent into merging (or splitting) two identities, which would collapse their visibility
        // classes (spec §Cross-platform identity is operator-asserted only).
        if relation == RelationName::SameAs && self.authority == Authority::Platform {
            return Err(MemoryError::MergeForbidden);
        }
        // A link from or to `self` modifies the self model — barred outside the control panel.
        self.guard_self(from)?;
        self.guard_self(to)?;
        // Operator-authored links carry control-panel provenance; the agent's own carry `Agent`.
        let source = match self.authority {
            Authority::Operator => LinkSource::Debugger,
            Authority::Platform => LinkSource::Agent,
        };
        self.touched.insert(from);
        self.touched.insert(to);
        self.buffer.push(if create {
            EventPayload::LinkCreated {
                from,
                to,
                relation,
                source,
            }
        } else {
            EventPayload::LinkRemoved { from, to, relation }
        });
        Ok(())
    }

    /// Reject a platform-authority write that touches `self`. The control panel (operator authority)
    /// is the only path permitted to edit `self`, so the self model cannot be forged from a
    /// conversation (spec §Imprint interview). `create("self")` needs no guard — it is already blocked
    /// by `NameExists`, since `self` is seeded at genesis.
    fn guard_self(&self, id: MemoryId) -> Result<(), MemoryError> {
        if self.authority == Authority::Platform && Some(id) == self.self_id {
            return Err(MemoryError::SelfWriteForbidden);
        }
        Ok(())
    }

    /// The write-time default visibility for content told by `told_by` on the memory `name`: private
    /// in a `#confidential` room (regardless of namespace), else the namespace/subject default.
    fn default_visibility(&self, name: &str, id: MemoryId, told_by: &Teller) -> Visibility {
        if self.confidential_context {
            Visibility::PrivateToTeller
        } else {
            default_visibility_named(name, id, told_by)
        }
    }

    /// Buffer a content entry and touch its memory.
    fn push_content(
        &mut self,
        id: MemoryId,
        text: String,
        told_by: Teller,
        visibility: Visibility,
    ) {
        self.touched.insert(id);
        self.buffer.push(EventPayload::MemoryContentAppended {
            id,
            entry_id: EntryId::generate(),
            asserted_at: self.clock.now(),
            text,
            told_by,
            told_in: self.told_in,
            visibility,
        });
    }

    /// Resolve a name to a memory id, consulting this block's pending creates/renames before the
    /// graph (read-your-writes).
    fn resolve(&self, name: &str) -> Result<Option<MemoryId>, GraphError> {
        for event in &self.buffer {
            match event {
                EventPayload::MemoryCreated { id, name: created } if created.as_str() == name => {
                    return Ok(Some(*id));
                }
                EventPayload::MemoryRenamed { id, new_name, .. } if new_name.as_str() == name => {
                    return Ok(Some(*id));
                }
                _ => {}
            }
        }
        Ok(self.graph.memory_by_name(name)?.map(|memory| memory.id))
    }

    /// Resolve a memory's name from this block's pending creates first, then the graph — so an
    /// append's write-time default visibility is computed even for a memory created earlier in the
    /// same block.
    fn resolve_name(&self, id: MemoryId) -> Result<Option<MemoryName>, GraphError> {
        let pending = self.buffer.iter().find_map(|event| match event {
            EventPayload::MemoryCreated { id: created, name } if *created == id => {
                Some(name.clone())
            }
            _ => None,
        });
        match pending {
            Some(name) => Ok(Some(name)),
            None => Ok(self.graph.memory_by_id(id)?.map(|memory| memory.name)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AppendOptions, Authority, MemoryBlock, MemoryError};
    use crate::{
        clock::ManualClock,
        event::{Cardinality, EventPayload, LinkSource, Teller, Visibility},
        graph::Graph,
        ids::{ConversationId, MemoryId, MemoryName, RelationName, Timestamp},
        store::{MemoryStore, Store},
    };

    /// A block over an empty in-memory graph and a conversation with no context — enough to exercise
    /// the write invariants directly, no Lua VM and no store materialization involved.
    fn block<'a>(
        graph: &'a Graph,
        clock: &'a ManualClock,
        teller: Teller,
        authority: Authority,
    ) -> MemoryBlock<'a> {
        MemoryBlock::new(graph, clock, teller, authority, ConversationId::generate()).unwrap()
    }

    /// A graph seeded with the `self` memory and the `created_by` and `same_as` relations — the
    /// minimum to exercise the self-write and merge guards, which key on the resolved `self` id and on
    /// the relation. Returns the graph and `self`'s id.
    fn graph_with_self() -> (Graph, MemoryId) {
        let mut store = MemoryStore::new();
        let self_id = MemoryId::generate();
        store
            .append(
                Timestamp::from_millis(1_000),
                vec![
                    EventPayload::MemoryCreated {
                        id: self_id,
                        name: MemoryName::new(MemoryName::SELF),
                    },
                    EventPayload::LinkTypeRegistered {
                        name: RelationName::CreatedBy,
                        inverse: RelationName::Created,
                        from_card: Cardinality::One,
                        to_card: Cardinality::Many,
                        symmetric: false,
                        reflexive: false,
                    },
                    EventPayload::LinkTypeRegistered {
                        name: RelationName::SameAs,
                        inverse: RelationName::SameAs,
                        from_card: Cardinality::Many,
                        to_card: Cardinality::Many,
                        symmetric: true,
                        reflexive: false,
                    },
                ],
            )
            .unwrap();
        let mut graph = Graph::open_in_memory().unwrap();
        graph.materialize_from(&store).unwrap();
        (graph, self_id)
    }

    #[test]
    fn create_rejects_a_duplicate_name() {
        let graph = Graph::open_in_memory().unwrap();
        let clock = ManualClock::new(Timestamp::from_millis(1_000));
        let mut block = block(&graph, &clock, Teller::Agent, Authority::Platform);
        block.create("topic/plan", None).unwrap();
        // Caught against the block's own pending create (read-your-writes), before any commit.
        let error = block.create("topic/plan", None).unwrap_err();
        assert!(matches!(error, MemoryError::NameExists(_)));
    }

    #[test]
    fn link_rejects_an_unregistered_relation() {
        let graph = Graph::open_in_memory().unwrap();
        let clock = ManualClock::new(Timestamp::from_millis(1_000));
        let mut block = block(&graph, &clock, Teller::Agent, Authority::Platform);
        let a = block.create("topic/a", None).unwrap();
        let b = block.create("topic/b", None).unwrap();
        let error = block
            .link(a, b, RelationName::Other("bogus_relation".into()))
            .unwrap_err();
        assert!(matches!(error, MemoryError::UnknownRelation(_)));
    }

    #[test]
    fn an_aside_about_another_person_defaults_private() {
        let graph = Graph::open_in_memory().unwrap();
        let clock = ManualClock::new(Timestamp::from_millis(1_000));
        let speaker = MemoryId::generate();
        let mut block = block(
            &graph,
            &clock,
            Teller::Participant(speaker),
            Authority::Platform,
        );
        // The speaker (the teller) is not the subject of person/phil, so the default is private.
        let phil = block.create("person/phil", None).unwrap();
        block
            .append(phil, "is being managed out", AppendOptions::default())
            .unwrap();

        let visibility = block
            .into_effects()
            .events
            .into_iter()
            .find_map(|event| match event {
                EventPayload::MemoryContentAppended { visibility, .. } => Some(visibility),
                _ => None,
            })
            .unwrap();
        assert_eq!(visibility, Visibility::PrivateToTeller);
    }

    #[test]
    fn platform_authority_cannot_write_self() {
        let (graph, self_id) = graph_with_self();
        let clock = ManualClock::new(Timestamp::from_millis(2_000));
        let mut block = block(&graph, &clock, Teller::Agent, Authority::Platform);
        let other = block.create("person/phil", None).unwrap();

        // Appending to self, and a link with self at either endpoint, are all barred.
        assert!(matches!(
            block
                .append(self_id, "I am sentient", AppendOptions::default())
                .unwrap_err(),
            MemoryError::SelfWriteForbidden
        ));
        assert!(matches!(
            block
                .link(self_id, other, RelationName::CreatedBy)
                .unwrap_err(),
            MemoryError::SelfWriteForbidden
        ));
        assert!(matches!(
            block
                .link(other, self_id, RelationName::CreatedBy)
                .unwrap_err(),
            MemoryError::SelfWriteForbidden
        ));
        assert!(matches!(
            block
                .unlink(self_id, other, RelationName::CreatedBy)
                .unwrap_err(),
            MemoryError::SelfWriteForbidden
        ));
    }

    #[test]
    fn operator_authority_may_write_self_and_links_carry_debugger() {
        let (graph, self_id) = graph_with_self();
        let clock = ManualClock::new(Timestamp::from_millis(2_000));
        let mut block = block(&graph, &clock, Teller::Agent, Authority::Operator);
        let phil = block.create("person/phil", None).unwrap();

        // The same writes that platform authority bars all succeed from the control panel.
        block
            .append(
                self_id,
                "I exist to keep Phil's memory.",
                AppendOptions::default(),
            )
            .unwrap();
        block.link(self_id, phil, RelationName::CreatedBy).unwrap();

        // The operator-authored link carries control-panel provenance, not the agent's own.
        let source = block
            .into_effects()
            .events
            .into_iter()
            .find_map(|event| match event {
                EventPayload::LinkCreated { source, .. } => Some(source),
                _ => None,
            })
            .unwrap();
        assert_eq!(source, LinkSource::Debugger);
    }

    #[test]
    fn platform_authority_cannot_assert_a_same_as_merge() {
        let (graph, _self_id) = graph_with_self();
        let clock = ManualClock::new(Timestamp::from_millis(2_000));
        let mut block = block(&graph, &clock, Teller::Agent, Authority::Platform);
        let dave = block.create("person/dave", None).unwrap();
        let dave_discord = block.create("person/dave@discord", None).unwrap();

        // Merging two identities — or splitting one — is operator-only, regardless of the endpoints.
        assert!(matches!(
            block
                .link(dave, dave_discord, RelationName::SameAs)
                .unwrap_err(),
            MemoryError::MergeForbidden
        ));
        assert!(matches!(
            block
                .unlink(dave, dave_discord, RelationName::SameAs)
                .unwrap_err(),
            MemoryError::MergeForbidden
        ));
    }

    #[test]
    fn operator_authority_may_assert_a_same_as_merge() {
        let (graph, _self_id) = graph_with_self();
        let clock = ManualClock::new(Timestamp::from_millis(2_000));
        let mut block = block(&graph, &clock, Teller::Agent, Authority::Operator);
        let dave = block.create("person/dave", None).unwrap();
        let dave_discord = block.create("person/dave@discord", None).unwrap();

        block
            .link(dave, dave_discord, RelationName::SameAs)
            .unwrap();
    }
}
