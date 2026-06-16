//! The block transaction: the agent's memory-mutation surface as one opaque, invariant-enforcing
//! object.
//!
//! A [`MemoryBlock`] accumulates the side-effect events of a single Lua block — creates, appends,
//! links — committed or discarded atomically by the caller. It owns the buffer and the touched set,
//! resolves reads against the graph overlaid with its own pending writes (read-your-writes), and is
//! the one place the write invariants live: name uniqueness, registered relations, and the
//! write-time visibility default (including the `#confidential`-room firming). The Lua layer
//! ([`crate::agent::lua`]) is a thin wrapper over this — it translates script calls into method calls and
//! never touches the buffer, the events, or the visibility rules directly.

use std::{collections::BTreeSet, sync::Arc};

use serde::Deserialize;

use crate::{
    engine::Engine,
    event::{Cardinality, EventPayload, LinkSource, Teller, Visibility},
    graph::{EntryView, GraphError, RelationView, TagVocabularyEntry},
    ids::{ConversationId, EntryId, MemoryId, MemoryName},
    time::{self, TemporalRef, Timestamp},
    vocabulary::{RelationName, TagName},
};

use super::visibility::{default_visibility_named, subject_participant};

/// Who is driving a block's writes. Operator authority is the console; it is the only path
/// permitted to edit `self`, and it authors its links as `Operator` rather than `Agent` (spec
/// §Imprint interview). Platform authority is an ordinary conversation turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Authority {
    Platform,
    Operator,
}

/// One block's in-progress memory mutations. Built fresh per block, mutated through its operations,
/// and consumed by [`MemoryBlock::into_effects`] to commit or discard.
///
/// The graph and clock are reached through a shared [`Engine`] handle rather than borrows, so the
/// block (and the Lua functions that drive it) is `'static` — the Lua API now runs through
/// `Lua::create_function` and `eval_async` rather than a scope, and a scoped borrow could not survive
/// that. The graph is locked transiently per read; no guard is ever held across an `.await`.
pub struct MemoryBlock {
    engine: Arc<Engine>,
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
    /// Who is present in the conversation — the set `memory.search` filters its hits against (spec
    /// §Visibility). Carried so the read path can reach it; writes do not use it.
    present_set: Vec<MemoryId>,
    buffer: Vec<EventPayload>,
    touched: BTreeSet<MemoryId>,
    aborted: Option<String>,
}

/// An addressable content entry handed back to the agent: its stable [`EntryId`] and its text. The
/// id lets the agent pass an entry to [`MemoryBlock::supersede`]; the Lua layer renders it as its
/// text so reading stays ergonomic.
pub struct EntryRef {
    pub entry_id: EntryId,
    pub text: String,
    /// How widely the entry may surface — so a read renders it self-describingly and the agent sees at
    /// a glance whether a fact is a confidence to hold (`PrivateToTeller`/`Exclude`) or freely shareable.
    pub visibility: Visibility,
    /// Who the entry is attributed to, resolved to a readable label ("person/erin", "you" for the
    /// agent's own note) — so a read shows where a fact came from, which is what tells the agent whose
    /// confidence it is.
    pub teller: String,
    /// Whether the entry is under an unresolved belief arbitration — a fact the agent recorded as
    /// contested and should surface as such rather than assert as settled. Lets a read advertise the
    /// dispute so the agent honors it when answering, instead of confidently picking one account.
    pub disputed: bool,
    /// When the fact occurs, if dated — so a read shows the date inline rather than leaving it in a
    /// structured field the agent must inspect or search for separately. `None` for an undated note.
    pub occurred_at: Option<TemporalRef>,
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
    /// A `tags.create` named a tag already in the vocabulary — creation forces a fresh purpose, so a
    /// collision is a teachable error (apply it, or change its purpose with `tags.describe`).
    TagExists(TagName),
    /// A `mem:tag`/`tags.describe` named a tag that was never created. Tags are a described, shared
    /// vocabulary, so they must be created (`tags.create`) before they are applied or re-described.
    UnknownTag(TagName),
    /// A `links.register` gave a cardinality that is neither "one" nor "many".
    BadCardinality(String),
    /// A platform-authority write tried to touch `self` — appending to it, or linking from or to it.
    /// Only the console (operator authority) may edit `self`.
    SelfWriteForbidden,
    /// A platform-authority write tried to assert or retract a `same_as` merge. Cross-platform
    /// identity is operator-asserted only — the agent never merges two identities on its own.
    MergeForbidden,
    /// An agent-authored entry about a person was written with no explicit visibility. Such a write
    /// has no protective default — the aside mechanism keys on a participant teller, not the agent —
    /// so it must classify the entry rather than fall silently to public (which is how a re-recorded
    /// confidence leaks).
    VisibilityRequired,
    /// A `calendar.*` query was given an argument that does not parse — a malformed `within` duration
    /// or a non-`YYYY-MM-DD` date.
    BadCalendarArg(String),
    /// A `supersede` named an entry that is not a live entry of the memory's `same_as` class — an
    /// unknown id, or one already superseded. The agent supersedes entries it read from the same
    /// memory, so this is a teachable misuse.
    UnknownEntry(EntryId),
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
            MemoryError::TagExists(name) => write!(
                f,
                "a tag named {:?} already exists; apply it with mem:tag, or change its purpose with \
                 tags.describe",
                name.as_str()
            ),
            MemoryError::UnknownTag(name) => write!(
                f,
                "unknown tag {:?}; create it first with tags.create(name, purpose)",
                name.as_str()
            ),
            MemoryError::BadCardinality(value) => {
                write!(f, "cardinality {value:?} must be \"one\" or \"many\"")
            }
            MemoryError::SelfWriteForbidden => {
                write!(f, "self can only be edited from the console")
            }
            MemoryError::MergeForbidden => {
                write!(f, "same_as merges can only be asserted from the console")
            }
            MemoryError::VisibilityRequired => write!(
                f,
                "set this entry's visibility explicitly — pass {{ visibility = \"public\" }} or \
                 {{ visibility = \"private\" }}; an agent-authored note about a person has no safe \
                 default"
            ),
            MemoryError::BadCalendarArg(arg) => write!(
                f,
                "could not read the calendar argument {arg:?}; use a duration like \"7 days\" or a \
                 date like \"2026-06-03\""
            ),
            MemoryError::UnknownEntry(entry) => write!(
                f,
                "no live entry {} on this memory; supersede an entry you read from it",
                entry.0
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
/// the speaker's; `visibility` forces the visibility instead of the write-time default; `occurred_at`
/// records the real-world time the entry is *about*, distinct from when it is recorded (spec §Time).
/// Deserialized straight from the Lua `opts` table — `occurred_at` is a tagged table (see
/// [`TemporalRef`]).
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct AppendOptions {
    pub by_agent: bool,
    pub visibility: Option<VisibilityChoice>,
    pub occurred_at: Option<TemporalRef>,
}

/// A link relation to register, deserialized straight from the `links.register` table. Cardinalities
/// arrive as the lowercase strings the spec advertises (`"one"` / `"many"`) and are parsed at the
/// block boundary; `symmetric` and `reflexive` default to `false`.
#[derive(Debug, Deserialize)]
pub struct RelationSpec {
    pub name: String,
    pub inverse: String,
    pub from_card: String,
    pub to_card: String,
    #[serde(default)]
    pub symmetric: bool,
    #[serde(default)]
    pub reflexive: bool,
}

/// Parse a cardinality from the lowercase string `links.register` advertises (`"one"` / `"many"`),
/// case-insensitively. A `Cardinality` serializes as `One`/`Many` on the wire, but the agent-facing
/// API speaks lowercase, so the two are reconciled here rather than by widening the wire format.
fn parse_cardinality(value: &str) -> Result<Cardinality, MemoryError> {
    match value.to_ascii_lowercase().as_str() {
        "one" => Ok(Cardinality::One),
        "many" => Ok(Cardinality::Many),
        _ => Err(MemoryError::BadCardinality(value.to_owned())),
    }
}

impl MemoryBlock {
    /// Open a block for `conversation`: resolve the context it writes in and whether that room is
    /// `#confidential`. Fails only on a graph read error (infrastructure), never on agent input.
    pub fn new(
        engine: Arc<Engine>,
        teller: Teller,
        authority: Authority,
        conversation: ConversationId,
        present_set: Vec<MemoryId>,
    ) -> Result<MemoryBlock, GraphError> {
        let (told_in, confidential_context, self_id) = {
            let graph = engine.graph.lock();
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
            (told_in, confidential_context, self_id)
        };
        Ok(MemoryBlock {
            engine,
            teller,
            authority,
            self_id,
            told_in,
            confidential_context,
            present_set,
            buffer: Vec::new(),
            touched: BTreeSet::new(),
            aborted: None,
        })
    }

    /// A handle to the shared backends and the present set this block runs under — the inputs the
    /// async `memory.search` needs (the embedder, vector index, graph, clock, settings, and visibility
    /// set). Returned together so the Lua layer can embed and search without holding the block lock.
    pub fn retrieval_handle(&self) -> (Arc<Engine>, Vec<MemoryId>) {
        (self.engine.clone(), self.present_set.clone())
    }

    /// Create a memory, optionally with a first content entry. The name must be free — a collision is
    /// a teachable error rejected before anything is buffered, so a duplicate `MemoryCreated` never
    /// reaches the log (where it would only fail at materialize, poisoning replay).
    pub fn create(&mut self, name: &str, content: Option<&str>) -> Result<MemoryId, MemoryError> {
        if self.resolve(name)?.is_some() {
            return Err(MemoryError::NameExists(MemoryName::new(name)));
        }
        let id = MemoryId::generate();
        // A first entry is told like any append: by the turn's teller, classified the same way (an
        // agent-authored first entry about a person must set its visibility). Resolve it before
        // buffering anything, so an unclassified write fails without leaving a half-created memory.
        let first_entry = match content {
            Some(text) => {
                let teller = self.teller.clone();
                let visibility = self.resolve_visibility(Some(name), id, &teller, None)?;
                Some((text.to_owned(), teller, visibility))
            }
            None => None,
        };
        self.touched.insert(id);
        self.buffer.push(EventPayload::MemoryCreated {
            id,
            name: MemoryName::new(name),
        });
        if let Some((text, teller, visibility)) = first_entry {
            // A created memory's first entry carries no occurrence; `occurred_at` arrives via
            // `mem:append`, matching the spec's `dave:append("...", { occurred_at = ... })` form.
            self.push_content(id, text, teller, visibility, None);
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
    ) -> Result<EntryId, MemoryError> {
        self.guard_self(id)?;
        let told_by = if opts.by_agent {
            Teller::Agent
        } else {
            self.teller.clone()
        };
        let name = self.resolve_name(id)?;
        let visibility = self.resolve_visibility(
            name.as_ref().map(MemoryName::as_str),
            id,
            &told_by,
            opts.visibility,
        )?;
        Ok(self.push_content(id, text.to_owned(), told_by, visibility, opts.occurred_at))
    }

    /// Supersede `old` with `new` on `id` — the agent corrected or retracted a fact, recording which
    /// entry replaces it (spec §Visibility → superseded entries are not live). Both must be live
    /// entries of `id`'s `same_as` class (a live read, so the lock layer holds the class). Buffers a
    /// `MemorySuperseded`; the superseded entry then drops from every live surface while remaining in
    /// history. Like an append, it is a write to `id`, so platform authority may not supersede a
    /// `self` entry.
    pub fn supersede(
        &mut self,
        id: MemoryId,
        old: EntryId,
        new: EntryId,
    ) -> Result<(), MemoryError> {
        self.guard_self(id)?;
        let live = self.live_class_entry_ids(id)?;
        if !live.contains(&old) {
            return Err(MemoryError::UnknownEntry(old));
        }
        if !live.contains(&new) {
            return Err(MemoryError::UnknownEntry(new));
        }
        self.touched.insert(id);
        self.buffer.push(EventPayload::MemorySuperseded {
            id,
            entry: old,
            superseded_by: new,
        });
        Ok(())
    }

    /// The memory's live content entries: its whole `same_as` class from the graph plus this block's
    /// pending appends, minus any superseded this block (read-your-writes). A traversing read, so it
    /// touches every class member, not just `id`. Each entry is addressable (by id) so the agent can
    /// hand one to [`MemoryBlock::supersede`].
    pub fn entries(&mut self, id: MemoryId) -> Result<Vec<EntryRef>, MemoryError> {
        let (members, graph_entries, disputed) = {
            let graph = self.engine.graph.lock();
            (
                graph.class_members(id)?,
                graph.class_entries(id)?,
                graph.disputed_entries(id)?,
            )
        };
        let members = self.touch_class(id, members);
        // A supersession buffered this block (not yet committed) must hide its target from this live
        // read too, so the agent sees the effect of a correction it just made.
        let pending_superseded = self.pending_superseded();
        let mut refs: Vec<EntryRef> = graph_entries
            .into_iter()
            .filter(|entry| !pending_superseded.contains(&entry.entry_id))
            .map(|entry| self.entry_ref(entry, &disputed))
            .collect();
        refs.extend(self.pending_entries(&members, &pending_superseded));
        Ok(refs)
    }

    /// The memory's entries including superseded ones, oldest first — the agent's `mem:history()` view
    /// (spec §Per-memory history), the read where history is the point and the live filter is bypassed.
    /// Like [`MemoryBlock::entries`], a class-traversing read over the graph plus this block's pending
    /// appends; pending supersessions are *not* applied, since history keeps the superseded entries.
    pub fn history(&mut self, id: MemoryId) -> Result<Vec<EntryRef>, MemoryError> {
        let (members, graph_entries, disputed) = {
            let graph = self.engine.graph.lock();
            (
                graph.class_members(id)?,
                graph.class_history(id)?,
                graph.disputed_entries(id)?,
            )
        };
        let members = self.touch_class(id, members);
        let mut refs: Vec<EntryRef> = graph_entries
            .into_iter()
            .map(|entry| self.entry_ref(entry, &disputed))
            .collect();
        refs.extend(self.pending_entries(&members, &BTreeSet::new()));
        Ok(refs)
    }

    /// The live members of `id`'s `same_as` class (including `id`), for the Lua lock layer to acquire
    /// the whole class before a traversing read (spec §Concurrency → class-wide locking). A lock-free
    /// read returning an owned list: it touches nothing itself — the traversing read it precedes records
    /// the class into the touched set — and the graph guard is released before it returns.
    pub fn class_members(&self, id: MemoryId) -> Result<Vec<MemoryId>, MemoryError> {
        Ok(self.engine.graph.lock().class_members(id)?)
    }

    /// The current time off the engine clock — the anchor the `calendar` date constructors build
    /// relative dates on, so the agent names an operation rather than computing a date.
    pub fn now(&self) -> Timestamp {
        self.engine.clock.now()
    }

    /// Memories with a concrete occurrence within `within` of now (e.g. `"7 days"`, `"2 weeks"`;
    /// defaults to 7 days), soonest first (spec §Calendar). A read, so the results are touched.
    pub fn upcoming(&mut self, within: Option<&str>) -> Result<Vec<MemoryId>, MemoryError> {
        let within_millis = match within {
            Some(text) => time::parse_duration_millis(text)
                .ok_or_else(|| MemoryError::BadCalendarArg(text.to_owned()))?,
            None => DEFAULT_UPCOMING_DAYS * time::MILLIS_PER_DAY,
        };
        let now = self.engine.clock.now().as_millis();
        self.occurrence_memories(
            Timestamp::from_millis(now),
            Timestamp::from_millis(now.saturating_add(within_millis)),
        )
    }

    /// Memories with a concrete occurrence on the civil day `date` (`YYYY-MM-DD`).
    pub fn on(&mut self, date: &str) -> Result<Vec<MemoryId>, MemoryError> {
        let (from, to) =
            time::day_window(date).ok_or_else(|| MemoryError::BadCalendarArg(date.to_owned()))?;
        self.occurrence_memories(Timestamp::from_millis(from), Timestamp::from_millis(to))
    }

    /// Memories that carry a recurring occurrence — a listing; instances are not expanded yet.
    pub fn recurring(&mut self) -> Result<Vec<MemoryId>, MemoryError> {
        let ids: Vec<MemoryId> = self
            .engine
            .graph
            .lock()
            .recurring_memories()?
            .into_iter()
            .map(|memory| memory.id)
            .collect();
        for id in &ids {
            self.touched.insert(*id);
        }
        Ok(ids)
    }

    /// The distinct memories with an occurrence in `[from, to]`, soonest first, touched as reads —
    /// both concrete occurrences and the next in-window instance of a recurring entry (spec §Recurring
    /// materialization), merged and ordered by instant so a weekly standup interleaves with one-offs.
    fn occurrence_memories(
        &mut self,
        from: Timestamp,
        to: Timestamp,
    ) -> Result<Vec<MemoryId>, MemoryError> {
        let mut items: Vec<(Timestamp, MemoryId)> = Vec::new();
        {
            let graph = self.engine.graph.lock();
            for (memory, entry) in graph.occurrences_in_window(from, to)? {
                if let Some(sort) = entry.occurred_sort {
                    items.push((sort, memory.id));
                }
            }
            for (instant, memory) in graph.recurring_in_window(from, to)? {
                items.push((instant, memory.id));
            }
        }
        items.sort_by_key(|(instant, _)| *instant);

        let mut seen = BTreeSet::new();
        let mut ordered = Vec::new();
        for (_, id) in items {
            if seen.insert(id) {
                ordered.push(id);
            }
        }
        for id in &ordered {
            self.touched.insert(*id);
        }
        Ok(ordered)
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

    /// Register a link relation, accessible thereafter under either label; re-registering an existing
    /// name updates it in place (the materializer upserts). The cardinality strings are parsed here, at
    /// the block boundary, so a bad value is a teachable error rather than a silent mis-store.
    pub fn register_relation(&mut self, spec: RelationSpec) -> Result<(), MemoryError> {
        let from_card = parse_cardinality(&spec.from_card)?;
        let to_card = parse_cardinality(&spec.to_card)?;
        self.buffer.push(EventPayload::LinkTypeRegistered {
            name: RelationName::new(spec.name),
            inverse: RelationName::new(spec.inverse),
            from_card,
            to_card,
            symmetric: spec.symmetric,
            reflexive: spec.reflexive,
        });
        Ok(())
    }

    /// Every registered relation (committed), for `links.list`. A plain read of the projection; this
    /// block's pending registrations are not yet reflected, like every other committed read.
    pub fn all_relations(&self) -> Result<Vec<RelationView>, MemoryError> {
        Ok(self.engine.graph.lock().all_relations()?)
    }

    /// A single registered relation by either label (committed), for `links.get`, or `None`.
    pub fn relation(&self, name: &str) -> Result<Option<RelationView>, MemoryError> {
        Ok(self.engine.graph.lock().relation(name)?)
    }

    /// Whether `relation` is registered under either label — checking this block's pending
    /// `LinkTypeRegistered`s (read-your-writes) before the committed registry, so a relation registered
    /// and linked within the same block is recognized (spec §Read-your-writes within a block).
    fn relation_registered(&self, relation: &RelationName) -> Result<bool, MemoryError> {
        let pending = self.buffer.iter().any(|event| {
            matches!(
                event,
                EventPayload::LinkTypeRegistered { name, inverse, .. }
                    if name == relation || inverse == relation
            )
        });
        if pending {
            return Ok(true);
        }
        Ok(self
            .engine
            .graph
            .lock()
            .relation(relation.as_str())?
            .is_some())
    }

    /// Create a tag with a one-line purpose. A tag's description is set only at creation; applying it
    /// never mutates it (spec §Tag operations). A name already in the vocabulary is a teachable error.
    pub fn create_tag(&mut self, name: TagName, description: &str) -> Result<(), MemoryError> {
        if self.tag_exists(&name)? {
            return Err(MemoryError::TagExists(name));
        }
        self.buffer.push(EventPayload::TagCreated {
            name,
            description: description.to_owned(),
        });
        Ok(())
    }

    /// Change an existing tag's one-line purpose. The tag must already exist — re-describing an
    /// unknown tag is a teachable error (create it first).
    pub fn describe_tag(&mut self, name: TagName, description: &str) -> Result<(), MemoryError> {
        if !self.tag_exists(&name)? {
            return Err(MemoryError::UnknownTag(name));
        }
        self.buffer.push(EventPayload::TagDescriptionChanged {
            name,
            new_description: description.to_owned(),
        });
        Ok(())
    }

    /// Apply a tag to a memory. The tag must be in the vocabulary (`tags.create` first) — applying an
    /// unknown tag is a teachable error, since a tag is a shared, described vocabulary rather than an
    /// ad-hoc label. Tagging is idempotent at the projection (`INSERT OR IGNORE`).
    pub fn tag(&mut self, id: MemoryId, tag: TagName) -> Result<(), MemoryError> {
        self.guard_self(id)?;
        if !self.tag_exists(&tag)? {
            return Err(MemoryError::UnknownTag(tag));
        }
        self.touched.insert(id);
        self.buffer
            .push(EventPayload::TagAppliedToMemory { memory: id, tag });
        Ok(())
    }

    /// Remove a tag from a memory. Idempotent — removing a tag the memory does not carry is a no-op
    /// at the projection — so it needs no vocabulary check.
    pub fn untag(&mut self, id: MemoryId, tag: TagName) -> Result<(), MemoryError> {
        self.guard_self(id)?;
        self.touched.insert(id);
        self.buffer
            .push(EventPayload::TagRemovedFromMemory { memory: id, tag });
        Ok(())
    }

    /// The whole tag vocabulary (committed), for `tags.list`. A plain read of the projection; this
    /// block's pending tag creations are not yet reflected, like every other committed read.
    pub fn all_tags(&self) -> Result<Vec<TagVocabularyEntry>, MemoryError> {
        Ok(self.engine.graph.lock().all_tags()?)
    }

    /// Whether `name` is a created tag — checking this block's pending `TagCreated`s (read-your-writes)
    /// before the committed vocabulary, so a tag created and applied within the same block is
    /// recognized.
    fn tag_exists(&self, name: &TagName) -> Result<bool, MemoryError> {
        let pending = self.buffer.iter().any(|event| {
            matches!(event, EventPayload::TagCreated { name: created, .. } if created == name)
        });
        if pending {
            return Ok(true);
        }
        Ok(self
            .engine
            .graph
            .lock()
            .tag_description(name.as_str())?
            .is_some())
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

    /// Drain the block's effects without consuming it. The block now lives behind a shared
    /// `Arc<Mutex<…>>` (so the Lua functions can hold `'static` handles to it), which cannot be
    /// `try_unwrap`ped while those function references survive in the VM, so the caller reclaims the
    /// effects through the lock instead. Leaves the block empty.
    pub fn take_effects(&mut self) -> BlockEffects {
        BlockEffects {
            events: std::mem::take(&mut self.buffer),
            touched: std::mem::take(&mut self.touched).into_iter().collect(),
            aborted: self.aborted.take(),
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
        if !self.relation_registered(&relation)? {
            return Err(MemoryError::UnknownRelation(relation));
        }
        // Cross-platform identity is operator-asserted only: a participant must not be able to steer
        // the agent into merging (or splitting) two identities, which would collapse their visibility
        // classes (spec §Cross-platform identity is operator-asserted only).
        if relation == RelationName::SameAs && self.authority == Authority::Platform {
            return Err(MemoryError::MergeForbidden);
        }
        // A link from or to `self` modifies the self model — barred outside the console.
        self.guard_self(from)?;
        self.guard_self(to)?;
        // Operator-authored links carry operator provenance; the agent's own carry `Agent`.
        let source = match self.authority {
            Authority::Operator => LinkSource::Operator,
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

    /// Reject a platform-authority write that touches `self`. The console (operator authority)
    /// is the only path permitted to edit `self`, so the self model cannot be forged from a
    /// conversation (spec §Imprint interview). `create("self")` needs no guard — it is already blocked
    /// by `NameExists`, since `self` is seeded at genesis.
    fn guard_self(&self, id: MemoryId) -> Result<(), MemoryError> {
        if self.authority == Authority::Platform && Some(id) == self.self_id {
            return Err(MemoryError::SelfWriteForbidden);
        }
        Ok(())
    }

    /// The visibility a content entry is written at, or a teachable failure. An explicit choice is
    /// honored verbatim. With none: a `#confidential` room firms everything private; otherwise an
    /// agent-authored entry about a *person* (a subject-bearing memory) has no protective default —
    /// the participant-aside mechanism keys on a participant teller, not the agent, so silently
    /// defaulting to public is how a re-recorded confidence leaks — and must be classified. Any other
    /// write (a participant teller, or a non-subject memory like `self`/`topic/*`) takes the
    /// namespace/subject default.
    fn resolve_visibility(
        &self,
        name: Option<&str>,
        id: MemoryId,
        told_by: &Teller,
        explicit: Option<VisibilityChoice>,
    ) -> Result<Visibility, MemoryError> {
        if let Some(choice) = explicit {
            return Ok(match choice {
                VisibilityChoice::Public => Visibility::Public,
                VisibilityChoice::Private => Visibility::PrivateToTeller,
            });
        }
        if self.confidential_context {
            return Ok(Visibility::PrivateToTeller);
        }
        let about_a_person = name.is_some_and(|name| subject_participant(name, id).is_some());
        if matches!(told_by, Teller::Agent) && about_a_person {
            return Err(MemoryError::VisibilityRequired);
        }
        Ok(match name {
            Some(name) => default_visibility_named(name, id, told_by),
            None => Visibility::Public,
        })
    }

    /// Record `id` and its `same_as` class as touched (a traversing read locks the whole class), and
    /// return the class as a set for membership tests against the pending buffer.
    fn touch_class(&mut self, id: MemoryId, members: Vec<MemoryId>) -> BTreeSet<MemoryId> {
        self.touched.insert(id);
        let mut set = BTreeSet::new();
        for member in members {
            self.touched.insert(member);
            set.insert(member);
        }
        set.insert(id);
        set
    }

    /// The entries this block has superseded but not yet committed — applied to the live reads so a
    /// correction's effect is visible within the block (read-your-writes).
    fn pending_superseded(&self) -> BTreeSet<EntryId> {
        self.buffer
            .iter()
            .filter_map(|event| match event {
                EventPayload::MemorySuperseded { entry, .. } => Some(*entry),
                _ => None,
            })
            .collect()
    }

    /// This block's pending content appends to any member of `members`, as entry refs, skipping any in
    /// `exclude` — the read-your-writes tail of a live or history entry read.
    fn pending_entries(
        &self,
        members: &BTreeSet<MemoryId>,
        exclude: &BTreeSet<EntryId>,
    ) -> Vec<EntryRef> {
        self.buffer
            .iter()
            .filter_map(|event| match event {
                EventPayload::MemoryContentAppended {
                    id,
                    entry_id,
                    text,
                    told_by,
                    visibility,
                    occurred_at,
                    ..
                } if members.contains(id) && !exclude.contains(entry_id) => Some(EntryRef {
                    entry_id: *entry_id,
                    text: text.clone(),
                    visibility: visibility.clone(),
                    teller: self.teller_label(told_by),
                    disputed: false,
                    occurred_at: occurred_at.clone(),
                }),
                _ => None,
            })
            .collect()
    }

    /// The [`EntryRef`] for an entry just appended this block (found in the buffer) — so `mem:append`
    /// can hand back a handle that renders with the same visibility and teller a read would show.
    pub fn entry_ref_by_id(&self, entry_id: EntryId) -> Option<EntryRef> {
        self.buffer.iter().find_map(|event| match event {
            EventPayload::MemoryContentAppended {
                entry_id: appended,
                text,
                told_by,
                visibility,
                occurred_at,
                ..
            } if *appended == entry_id => Some(EntryRef {
                entry_id: *appended,
                text: text.clone(),
                visibility: visibility.clone(),
                teller: self.teller_label(told_by),
                disputed: false,
                occurred_at: occurred_at.clone(),
            }),
            _ => None,
        })
    }

    /// Project an [`EntryView`] into an [`EntryRef`], resolving its teller to a readable label and
    /// marking it disputed when it is in the memory's set of unresolved-arbitration competing entries.
    fn entry_ref(&self, view: EntryView, disputed: &BTreeSet<EntryId>) -> EntryRef {
        EntryRef {
            disputed: disputed.contains(&view.entry_id),
            entry_id: view.entry_id,
            text: view.text,
            visibility: view.visibility,
            teller: self.teller_label(&view.told_by),
            occurred_at: view.occurred_at,
        }
    }

    /// A readable label for who an entry is attributed to: the participant's canonical handle, `you`
    /// for the agent's own observations, or `genesis` for seeded content.
    fn teller_label(&self, teller: &Teller) -> String {
        match teller {
            Teller::Participant(id) => self
                .resolve_name(*id)
                .ok()
                .flatten()
                .map(|name| name.as_str().to_owned())
                .unwrap_or_else(|| "someone".to_owned()),
            Teller::Agent => "you".to_owned(),
            Teller::Bootstrap => "genesis".to_owned(),
        }
    }

    /// The live entry ids of `id`'s `same_as` class: committed-live (the graph already excludes
    /// committed supersessions) plus this block's pending appends, minus what it has superseded —
    /// the set [`MemoryBlock::supersede`] validates its arguments against.
    fn live_class_entry_ids(&self, id: MemoryId) -> Result<BTreeSet<EntryId>, MemoryError> {
        let (members, committed) = {
            let graph = self.engine.graph.lock();
            (graph.class_members(id)?, graph.class_entries(id)?)
        };
        let members: BTreeSet<MemoryId> = members.into_iter().chain([id]).collect();
        let pending_superseded = self.pending_superseded();
        let mut ids: BTreeSet<EntryId> = committed
            .into_iter()
            .map(|entry| entry.entry_id)
            .filter(|entry_id| !pending_superseded.contains(entry_id))
            .collect();
        for entry in self.pending_entries(&members, &pending_superseded) {
            ids.insert(entry.entry_id);
        }
        Ok(ids)
    }

    /// Buffer a content entry and touch its memory, returning the minted entry id (so a write can be
    /// handed back to the agent as an addressable entry — see [`MemoryBlock::append`]).
    fn push_content(
        &mut self,
        id: MemoryId,
        text: String,
        told_by: Teller,
        visibility: Visibility,
        occurred_at: Option<TemporalRef>,
    ) -> EntryId {
        let entry_id = EntryId::generate();
        self.touched.insert(id);
        self.buffer.push(EventPayload::MemoryContentAppended {
            id,
            entry_id,
            asserted_at: self.engine.clock.now(),
            occurred_at,
            text,
            told_by,
            told_in: self.told_in,
            visibility,
        });
        entry_id
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
        Ok(self
            .engine
            .graph
            .lock()
            .memory_by_name(name)?
            .map(|memory| memory.id))
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
            None => Ok(self
                .engine
                .graph
                .lock()
                .memory_by_id(id)?
                .map(|memory| memory.name)),
        }
    }
}

const DEFAULT_UPCOMING_DAYS: i64 = 7;

#[cfg(test)]
mod tests;
