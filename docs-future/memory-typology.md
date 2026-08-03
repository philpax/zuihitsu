# The memory typology

Memory is four kinds with four lifecycles. The kind is a type distinction, not a label: each has its own authority over who may write it, its own retrieval rule, its own decay, and its own relationship to truth.

Forcing four kinds through one visibility predicate, one similarity threshold, and one decay function is wrong for three of them.

## Semantic

Curated claims about the world: the [Statements](statements.md) the rest of this tree describes.

Durable, credence-bearing, audience-gated, and distilled into the descriptions that summarise a memory. Written by the agent during a turn and by maintenance passes afterwards. This is the kind that means what it says.

## Episodic

The occasions themselves: what was said, by whom, in what order, and what it was like.

Episodic memory is **raw experience, not asserted truth**, and the distinction is the whole reason it is a separate kind. "Someone said X" and "X is true" are different facts, and a system that lets the first become the second by consolidation has laundered a claim into a belief.

The rules follow from that:

- Always told by the agent, never by a participant.
- Never a premise in a derivation.
- Never distilled into another memory's description.
- Never accrues attestation or corroboration.
- Marked as a reconstruction wherever it surfaces.

Retrieval is the part that differs most from the current design. An episode is a **linked companion** to the Statements recorded during it, not a fallback consulted when semantic search misses. Each knows the other structurally, so surfacing one surfaces the other without a second search. The experimental evidence is specific on this point: the gain lives where both traces are present and their anchors can be cross-referenced, and it is exactly zero where a single lookup suffices. A fallback tier would be consulted precisely when the pair is least useful.

Episodes decay by recency in ranking, but they are not retired. An old episode is not wrong, merely distant.

Not every session earns one. See [the two traces](two-traces.md) for why a low rate is correct rather than a coverage problem to tune away.

## Procedural

Executable Luau the agent has saved: a routine it worked out once and can invoke again.

Indexed by a natural-language description embedding, so it is found by what it does rather than by what it is called. Retrieved on demand rather than held in the prompt, which is what keeps the surface from growing with every saved routine. Invoked in the same frozen sandbox under the same step and timeout budget as any other block, with no additional authority.

Decay is by **invocation**, not by calendar age. A correct routine that has not been needed for months is not stale the way a fact about someone's job is stale; it is simply unused. Ranking on recency and frequency of use captures this, and it has the useful property of being independent of any embedding model and deterministic under replay.

Procedures are produced deliberately, and may also be produced automatically from a deliberation that turned out to be costly or repeated. A turn that required unusually deep multi-step reasoning is a candidate for being saved as a routine, which is the same instinct as compiling a hard-won result rather than re-deriving it.

## Working

A private scratchpad: persistent but transient, the agent's own head.

Outside the visibility model entirely. It has no audience because it has no readers, no teller because nothing told it, and no credence because it asserts nothing. It is the staging area where a thought is held long enough to become something or be discarded.

A reflection pass promotes or discards. Promotion means writing an actual Statement, with all the provenance and posture that entails. Discarding means the note is gone.

How the scratchpad is stored is genuinely unresolved: keeping it in the log preserves the commitment that the system is a pure function of its log, but scratchpad churn is exactly the transient state that should not bloat replay. A compactable channel whose net effect after reflection is a single promote-or-discard is the leading candidate, and a side table is a defensible alternative. Recorded as open in [`confidence.md`](confidence.md).

## Directives are not memory

Instructions about how to behave in a context, and the agent's own charter, are **configuration**. They live outside the typology.

They have no teller, no truth value, no credence, no validity interval, and no audience. Nothing about the Statement machinery applies to them, and putting them in the same container as facts is a category error that costs real capacity: [the corpus study](research/2026-08-03/modelling-study.md) found twenty-two such entries filed as ordinary content in a 198-entry corpus, one of them repeated verbatim ten times because each re-mint re-appended it.

## Ingesting a long document

Bulk ingestion falls out of the typology rather than needing its own machinery.

A long source lands as **semantic Statement clusters**, with sections and claims becoming Events and Statements linked by composition and summarisation relations, giving the work a navigable structure rather than one undifferentiated blob. Alongside it, an **episodic source layer** retains each span as the gloss its Statements point at, so any extracted claim traces back to the text that produced it.

The observed-against-recorded split is what makes this coherent: a document written years ago and ingested today records both truthfully.

Not everything in a document warrants extraction. The gate is the same judgement the episode rate needs, and the same discipline applies: retaining raw experience cheaply and structuring selectively is what keeps the cost per fact falling as the store grows.
