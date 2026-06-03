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

/// One block's in-progress memory mutations. Built fresh per block, mutated through its operations,
/// and consumed by [`MemoryBlock::into_effects`] to commit or discard.
pub struct MemoryBlock<'a> {
    graph: &'a Graph,
    clock: &'a dyn Clock,
    /// The turn's teller, attributed to content written this block unless an append opts out.
    teller: Teller,
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
    UnknownRelation(String),
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
                "unknown relation {relation:?}; it must be a registered link type"
            ),
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
        conversation: ConversationId,
    ) -> Result<MemoryBlock<'a>, GraphError> {
        let told_in = graph.context_for_conversation(conversation)?;
        let confidential_context = match told_in {
            Some(context_id) => graph
                .memory_by_id(context_id)?
                .is_some_and(|context| context.tags.contains(&TagName::Confidential)),
            None => false,
        };
        Ok(MemoryBlock {
            graph,
            clock,
            teller,
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
        relation: &str,
    ) -> Result<(), MemoryError> {
        self.change_link(from, to, relation, true)
    }

    /// Remove such a link.
    pub fn unlink(
        &mut self,
        from: MemoryId,
        to: MemoryId,
        relation: &str,
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
        relation: &str,
        create: bool,
    ) -> Result<(), MemoryError> {
        if self.graph.relation(relation)?.is_none() {
            return Err(MemoryError::UnknownRelation(relation.to_owned()));
        }
        let relation = RelationName::new(relation);
        self.touched.insert(from);
        self.touched.insert(to);
        self.buffer.push(if create {
            EventPayload::LinkCreated {
                from,
                to,
                relation,
                source: LinkSource::Agent,
            }
        } else {
            EventPayload::LinkRemoved { from, to, relation }
        });
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
    use super::{AppendOptions, MemoryBlock, MemoryError};
    use crate::{
        clock::ManualClock,
        event::{EventPayload, Teller, Visibility},
        graph::Graph,
        ids::{ConversationId, MemoryId, Timestamp},
    };

    /// A block over an empty in-memory graph and a conversation with no context — enough to exercise
    /// the write invariants directly, no Lua VM and no store materialization involved.
    fn block<'a>(graph: &'a Graph, clock: &'a ManualClock, teller: Teller) -> MemoryBlock<'a> {
        MemoryBlock::new(graph, clock, teller, ConversationId::generate()).unwrap()
    }

    #[test]
    fn create_rejects_a_duplicate_name() {
        let graph = Graph::open_in_memory().unwrap();
        let clock = ManualClock::new(Timestamp::from_millis(1_000));
        let mut block = block(&graph, &clock, Teller::Agent);
        block.create("topic/plan", None).unwrap();
        // Caught against the block's own pending create (read-your-writes), before any commit.
        let error = block.create("topic/plan", None).unwrap_err();
        assert!(matches!(error, MemoryError::NameExists(_)));
    }

    #[test]
    fn link_rejects_an_unregistered_relation() {
        let graph = Graph::open_in_memory().unwrap();
        let clock = ManualClock::new(Timestamp::from_millis(1_000));
        let mut block = block(&graph, &clock, Teller::Agent);
        let a = block.create("topic/a", None).unwrap();
        let b = block.create("topic/b", None).unwrap();
        let error = block.link(a, b, "bogus_relation").unwrap_err();
        assert!(matches!(error, MemoryError::UnknownRelation(_)));
    }

    #[test]
    fn an_aside_about_another_person_defaults_private() {
        let graph = Graph::open_in_memory().unwrap();
        let clock = ManualClock::new(Timestamp::from_millis(1_000));
        let speaker = MemoryId::generate();
        let mut block = block(&graph, &clock, Teller::Participant(speaker));
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
}
