//! The memory-block error type — the teachable violations and infrastructure failures a block
//! operation surfaces, delegating its message to the underlying cause.

use crate::{
    graph::GraphError,
    ids::{EntryId, MemoryName},
    vocabulary::{RelationName, TagName},
};

/// A write that violates an invariant, surfaced to the agent as a teachable error, or an underlying
/// graph read failure. The teachable variants' `Display` is the agent-facing message the Lua layer
/// renders as the block's terminal cause, so they are deliberately unprefixed — the agent reads
/// them, not an operator. The [`Graph`](MemoryError::Graph) variant is infrastructure (the Lua layer
/// intercepts it and surfaces a generic "internal graph error" to the agent, stashing the real error
/// for the operator), so it carries a `memory:` context prefix per the error-display convention,
/// nesting the graph error's own `materialized graph (…)` prefix.
#[derive(Debug)]
pub enum MemoryError {
    /// A `create` or `rename` collided with an existing name (names are unique). `similar` carries the
    /// near-matching existing handles in the same namespace, closest first, so the teachable error can
    /// point the agent at the neighbours and it chooses a distinguishing name rather than colliding
    /// again; it is empty when the handle is in no namespace or has no near-matches.
    NameExists {
        name: MemoryName,
        similar: Vec<MemoryName>,
    },
    /// A `link`/`unlink` named a relation that is not a registered link type.
    UnknownRelation(RelationName),
    /// A `tags.create` named a tag already in the vocabulary — creation forces a fresh purpose, so a
    /// collision is a teachable error (apply it, or change its purpose with `tags.describe`). `similar`
    /// carries the near-matching existing tags, closest first, so the error can surface a near-duplicate
    /// the agent may have meant; it is empty when there are no near-matches.
    TagExists {
        name: TagName,
        similar: Vec<TagName>,
    },
    /// A `mem:tag`/`tags.describe` named a tag that was never created. Tags are a described, shared
    /// vocabulary, so they must be created (`tags.create`) before they are applied or re-described.
    UnknownTag(TagName),
    /// A `links.register` gave a cardinality that is neither "one" nor "many".
    BadCardinality(String),
    /// A platform-authority write tried to touch `self` — appending to it, or linking from or to it.
    /// Only the console (operator authority) may edit `self`.
    SelfWriteForbidden,
    /// A write tried to record content on `person/operator`, the operator's provisional identity
    /// anchor. It holds no content of its own — facts about the operator belong on their real
    /// `person/<name>` profile, which is merged into it — so the anchor stays a pure merge target.
    OperatorWriteForbidden,
    /// A platform-authority write tried to assert or retract a `same_as` merge directly. The agent
    /// never authors a `same_as` from a turn — it `propose_merge`s, and the adjudication pass (or the
    /// operator) decides; a retraction is operator-only.
    MergeForbidden,
    /// A merge proposal named the same memory twice — there is nothing to merge.
    MergeProposalInvalid,
    /// An agent-authored entry about a person was written with no explicit visibility. Such a write
    /// has no protective default — the aside mechanism keys on a participant teller, not the agent —
    /// so it must classify the entry rather than fall silently to public (which is how a re-recorded
    /// confidence leaks).
    VisibilityRequired,
    /// A `set_volatility` named a level that is not `low`, `medium`, or `high`.
    UnknownVolatility(String),
    /// An append or link set both `visibility` and `exclude`. An exclude *is* a private posture (a
    /// confidence additionally withheld when a named party is present), so it takes no separate
    /// `visibility` — the two together are contradictory, a teachable error rather than a silent
    /// precedence.
    VisibilityConflict,
    /// An `exclude` named no one — an empty list. An exclude withheld from nobody is just a confidence
    /// for its teller, so the write is rejected pointing at `visibility = "private"` for that case.
    ExcludeEmpty,
    /// An `exclude` append targeted a memory whose same-block seed entry (the `create(name, content)`
    /// argument) took the *unforced* write-time default and landed open. The open seed beside the
    /// guard is the one plain copy that undoes it, so the append is rejected for the agent to reissue
    /// the block with the seed classified too — or created bare, every detail under the guard. An
    /// explicitly classified seed never trips this: a deliberate `visibility = "public"` is the
    /// agent's own call.
    UnguardedSeedBesideExclude,
    /// A `calendar.*` query was given an argument that does not parse — a malformed `within` duration
    /// or a non-`YYYY-MM-DD` date.
    BadCalendarArg(String),
    /// An `occurred_at` recurrence is not a rule this build can interpret — a free-phrased cadence
    /// such as "every Monday" rather than an RFC 5545 rule with a supported `FREQ`. Such a rule would
    /// arm a wake-up no one can derive, so the write is rejected for the agent to reissue correctly.
    UnsupportedRecurrence(String),
    /// A `supersede` named an entry that is not a live entry of the memory's `same_as` class — an
    /// unknown id, or one already superseded. The agent supersedes entries it read from the same
    /// memory, so this is a teachable misuse.
    UnknownEntry(EntryId),
    /// A platform-authority turn tried to remove the `#confidential` tag. The teller-private marker
    /// resolves a room's `#confidential` flag at read time, so removing the tag retroactively weakens
    /// the disclosure-judgment signal on every historical aside told under it — a broadcast, retroactive
    /// change with no legitimate platform-turn use, so it is barred outside the console (mirroring the
    /// `self`-write rationale, spec §Trust model). Adding the tag stays ungated: adding is conservative.
    ConfidentialUntagForbidden,
    /// A platform-authority turn tried to supersede another participant's confidence — a non-public
    /// entry (`PrivateToTeller`/`Exclude`) told by a participant who is not the current speaker (nor a
    /// merged identity of theirs). Superseding it suppresses what someone else entrusted, so it is
    /// reserved for its own teller's turn or the console (spec §Trust model). Its `Display` deliberately
    /// does not name the foreign teller — who confided the fact is itself part of the confidence.
    ForeignConfidenceSupersedeForbidden,
    /// A content entry exceeded the maximum length. Memory entries record distilled facts, not source
    /// content — the agent should summarize what it learned in under the limit rather than pasting a
    /// fetched page or raw transcript.
    ContentTooLong { length: usize, limit: usize },
    /// A graph read failed — infrastructure, not the agent's doing.
    Graph(GraphError),
}

impl std::fmt::Display for MemoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemoryError::NameExists { name, similar } => {
                write!(
                    f,
                    "a memory named {:?} already exists; fetch it with memory.get, or use \
                     memory.get_or_create(name) when you are unsure whether it exists",
                    name.as_str()
                )?;
                write_similar(f, "handles", similar.iter().map(MemoryName::as_str))
            }
            MemoryError::UnknownRelation(relation) => write!(
                f,
                "unknown relation {:?}; register it with links.register first, or call links.list \
                 for the known relations",
                relation.as_str()
            ),
            MemoryError::TagExists { name, similar } => {
                write!(
                    f,
                    "a tag named {:?} already exists; apply it with mem:tag, or change its purpose \
                     with tags.describe",
                    name.as_str()
                )?;
                write_similar(f, "tags", similar.iter().map(TagName::as_str))
            }
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
            MemoryError::OperatorWriteForbidden => {
                write!(
                    f,
                    "person/operator is a provisional anchor and holds no content; record what you \
                     learn about the operator on their real person/<name> profile, which is merged \
                     into it"
                )
            }
            MemoryError::MergeProposalInvalid => {
                write!(f, "a merge proposal must name two different memories")
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
            MemoryError::UnknownVolatility(level) => write!(
                f,
                "unknown volatility {level:?}; use \"low\", \"medium\", or \"high\""
            ),
            MemoryError::VisibilityConflict => write!(
                f,
                "set either visibility or exclude, not both — an exclude is already a private posture \
                 (withheld from its teller's audience, and additionally whenever a named party is \
                 present), so it takes no separate visibility"
            ),
            MemoryError::ExcludeEmpty => write!(
                f,
                "exclude must name at least one person to withhold this from — pass the handles or \
                 names, e.g. exclude = {{ \"person/dave\" }}; to keep it a plain confidence for its \
                 teller alone, use visibility = \"private\" instead"
            ),
            MemoryError::UnguardedSeedBesideExclude => write!(
                f,
                "this memory's creation content took the default visibility, so it would sit in the \
                 open beside the excluded entry you are adding — one plain copy undoes the guard. \
                 Reissue the block: create the memory bare and append every detail with exclude, or \
                 classify the creation content explicitly in the create opts (visibility or exclude)"
            ),
            MemoryError::BadCalendarArg(arg) => write!(
                f,
                "could not read the calendar argument {arg:?}; use a duration like \"7 days\", \
                 \"2 weeks\", or \"6 months\", or a date like \"2026-06-03\""
            ),
            MemoryError::UnsupportedRecurrence(rule) => write!(
                f,
                "the recurrence {rule:?} is not a supported rule; use an RFC 5545 rule with a \
                 supported FREQ, e.g. {{ recurring = \"FREQ=WEEKLY;BYDAY=MO\" }} for every Monday"
            ),
            MemoryError::UnknownEntry(entry) => write!(
                f,
                "no live entry {} on this memory; supersede an entry you read from it",
                entry.0
            ),
            MemoryError::ConfidentialUntagForbidden => write!(
                f,
                "removing #confidential retroactively changes how every aside told under it is marked, \
                 so it can only be cleared from the console; if the room is no longer confidential, \
                 ask the operator"
            ),
            MemoryError::ForeignConfidenceSupersedeForbidden => write!(
                f,
                "this is another participant's confidence; if the fact is out of date, append a \
                 correction as a new entry rather than superseding what someone else entrusted — \
                 superseding it is for its own teller's turn, or the console"
            ),
            MemoryError::ContentTooLong { length, limit } => write!(
                f,
                "this entry is {length} characters; memory entries record distilled facts, not source \
                 content — summarize what you learned in under {limit} characters, drawing on the \
                 content you fetched or read in your tool results rather than pasting it verbatim"
            ),
            MemoryError::Graph(error) => write!(f, "memory: {error}"),
        }
    }
}

/// Append a "similar existing …" clause naming the near-matching `names` (their kind, e.g. `handles`
/// or `tags`), or nothing when there are none — the collision error lists the neighbours so the agent
/// picks a distinguishing name rather than colliding again.
fn write_similar<'a>(
    f: &mut std::fmt::Formatter<'_>,
    kind: &str,
    names: impl ExactSizeIterator<Item = &'a str>,
) -> std::fmt::Result {
    if names.len() == 0 {
        return Ok(());
    }
    write!(f, "; similar existing {kind}: ")?;
    for (i, name) in names.enumerate() {
        if i > 0 {
            write!(f, ", ")?;
        }
        write!(f, "{name}")?;
    }
    write!(
        f,
        " — pick a distinguishing name if you mean a different one"
    )
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
