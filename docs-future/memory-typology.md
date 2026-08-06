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
- Composed from public content only, because a narrative that recounts a confidence is a second read path with no audience computed on it. See [the two traces](two-traces.md) for what this costs.

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

A [reflection pass](off-turn.md) promotes or discards. Promotion means writing an actual Statement, with all the provenance and audience condition that entails. Discarding means the note is gone.

### Promotion carries a taint

A note has no transmission principle of its own, which is what makes promotion dangerous: the [audience-invariant critic](the-seam.md) checks that no endorsement is wider than what it was founded under, and a note founded under nothing passes trivially. A confidence reasoned about in the scratchpad and then promoted arrives with whatever audience the promotion chose.

So a note carries a **taint set**: the Statements consulted while it was written. Promotion intersects the promoted Statement's principle against them, which is the same arithmetic [a derivation](statements.md) already does over its premises, reaching one step further back.

Two consequences the design should own rather than discover.

**Taint is monotone, so it must be per note.** A taint set that accumulates across a whole deliberation converges on everything the agent read, and promotion becomes impossible: everything is tainted by the strictest thing in the session. Tainting each note with what *that note* consulted keeps the sets small enough to be useful. A note that genuinely draws on a confidence should be hard to promote publicly; a note written beside one should not.

**It settles how the scratchpad is stored.** Keeping notes in the log preserves the commitment that the system is a pure function of its log, while a side table would keep transient churn out of replay. A taint set decides it: the set is state the fold must reproduce, so a side table is not available. What remains is a compactable channel whose net effect after reflection is a single promote-or-discard. This resolves an [open question](confidence.md) toward the option the research lane was least sure of, and it is the audience invariant rather than a storage argument that forces it.

## The self is not a memory

The agent's identity, its voice, its charter, and the standing instructions it operates under are **configuration**. They live in a dedicated slot outside the typology, not in any of the four kinds.

They have no teller, no truth value, no credence, no validity interval, and no audience. Nothing about the Statement machinery applies to them, and putting them in the same container as facts is a category error that costs real capacity: [the corpus study](research/2026-08-03/modelling-study.md) found twenty-two directive entries filed as ordinary content in a 198-entry corpus, one of them repeated verbatim ten times because each re-mint re-appended it.

### The slot

The self slot is an append-only sequence of versions, of which one is current. It is:

- **Always in context.** It is not retrieved, so it cannot fail to be retrieved. No ranking, no similarity, no budget under which it loses to something more recent.
- **Invisible to the memory API.** It is not returned by search, not readable or writable through the memory verbs, and cannot be retracted, consolidated, distilled, superseded, or tombstoned. No pass can reach it.
- **Operator-owned.** A new version is an operator write. The agent may *propose* one, and a proposal is an ordinary Statement about the agent that reaches the [exception queue](the-seam.md), not an edit.
- **Versioned rather than mutable**, so a change to who the agent is has a date, an author, and a diff, and the prompt reads exactly one version.

The current system keeps the charter as immutable content entries on a `self` memory, which protects the wording and not the slot. Immutability stops an entry being rewritten; it does not stop the entry being retracted, selected into a synthesis by a consolidation pass, summarised into a regenerable description, or dropped from a surface by an audience evaluation. A thing that must appear in every prompt should not be reachable by the machinery whose entire purpose is deciding what to leave out.

### What stays on the memory side

Claims *about* the agent are ordinary Statements with the agent as their subject: what it did, what it noticed about itself, what someone told it about how it comes across. Those carry tellers, accrue credence, can be contradicted, and can be wrong, and the agent writes them as [an ordinary fallible teller](privacy-and-provenance.md). The slot holds only what the agent is by construction, which is not the sort of thing that can be corroborated.

The [frame](statements.md) falls on the memory side of this line, and the distinction is worth keeping sharp. A persona agent's stated opinions are Statements in the `persona` frame; the instruction to speak in that voice is configuration. The first can be learned, superseded, and disputed. The second is a decision someone made.

### Why the agent does not hold the pen

A charter the agent can edit drifts without bound, because the drift is self-reinforcing: the next turn reads what the last turn wrote, and there is no outside signal correcting it. This is the same argument that keeps [credence](belief.md) off the writer and [merges](identity.md) below the wall, applied to the one piece of state that conditions every other decision the agent makes.

The cost is real and worth naming. An agent that cannot revise its own charter cannot grow into a different one on its own initiative, and every such change costs operator attention. That is the trade, taken deliberately, and it is the same exception-triggered-attention posture the rest of the design takes.

## Ingesting a long document

Bulk ingestion lands *in* the typology without needing new kinds, and it does need its own machinery. The earlier claim that it needs none was wrong, and the arithmetic says so plainly.

A long source lands as **semantic Statement clusters**, with sections and claims becoming Events and Statements linked by composition and summarisation relations, giving the work a navigable structure rather than one undifferentiated blob. Alongside it, an **episodic source layer** retains each span as the gloss its Statements point at, so any extracted claim traces back to the text that produced it.

The observed-against-recorded split is what makes this coherent: a document written years ago and ingested today records both truthfully.

Not everything in a document warrants extraction. The gate is the same judgement the episode rate needs, and the same discipline applies: retaining raw experience cheaply and structuring selectively is what keeps the cost per fact falling as the store grows.

### Why it needs a path of its own

The [write surface](write-surface.md) structures one utterance per extraction call and writes the call and its response to the log. Against the numbers [`evolution.md`](evolution.md) stage 0c measures, a hundred-thousand-word source chunked into a few hundred spans is a few hundred calls, tens of megabytes of log, and tens of minutes of wall clock before its p90 tail is counted. A document is not a long conversation, and treating it as one violates the fourth commitment directly.

Three properties are therefore owed by a bulk path, and none of them is expressible as a loop over `record`:

- **Extraction batches many spans per call**, so cost scales with the document rather than with its chunk count.
- **The gate runs before extraction, not as part of it.** A judgement about what warrants structuring that is itself a model call per span reduces writes without reducing calls, which is the wrong half.
- **Failure is per span and never loses the source.** A span that will not structure stays a gloss under its source layer, exactly as an utterance does.

Deferring the structuring to a later pass is not an option, for the reason [`write-surface.md`](write-surface.md) gives: structure recovered from committed prose is the tax this design exists to end, and an exception for documents is an exception for the largest inputs there are.

This is a real gap. The path is named and its constraints are stated; the mechanism is not designed, and the cost model belongs beside stage 0c's numbers rather than in a paragraph here.
