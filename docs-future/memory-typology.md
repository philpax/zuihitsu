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
- Composed under the intersection rule, and never over a confidence, because a narrative body cannot be partially surfaced. See [the two traces](two-traces.md).

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

**Consulted is defined operationally**, and this is the part that decides whether the mechanism works at all. The set is **what left a read event**: the explicit reads performed by the block that wrote the note, plus the turn's ambient recall, both of which are recorded and foldable. The brief is excluded, not because it is less present to the model but because it leaves no read event and is itself audience-computed before composition, so tainting against it would taint every note with everything on the first note and end promotion immediately.

The line is a proxy and should be read as one. A model cannot distinguish what a note *drew on* from what was merely in front of it while it was written, so ambient recall counts even where the agent never asked for it: a note written beside a confidence is tainted as surely as one written from it. That is over-tainting, and it is the acceptable direction, because the cost is a note that must wait for corroboration rather than a confidence that escapes. It is also survivable in practice, because taint governs only the promotion of notes: what the agent may assert from a live conversation runs through the ordinary [write surface](write-surface.md) with its own provenance and is untouched by any of this.

Two consequences the design should own rather than discover.

**Taint is monotone, so it must be per note.** A taint set that accumulates across a whole deliberation converges on everything the agent read, and promotion becomes impossible: everything is tainted by the strictest thing in the session. Tainting each note with what *that note* saw keeps the sets small. On the live instance the reads a single block performs run to a median of one memory and a maximum of eleven, which is the closest available proxy: it is measured on a system that has no scratchpad, so it bounds block reads rather than note consultation, and it is a reason to expect the mechanism to be affordable rather than a demonstration that it is.

**It settles how the scratchpad is stored.** Keeping notes in the log preserves the commitment that the system is a pure function of its log, while a side table would keep transient churn out of replay. A taint set decides it: the set is state the fold must reproduce, so a side table is not available. What remains is a compactable channel whose net effect after reflection is a single promote-or-discard.

The volume worry that motivated the side table does not survive contact with the numbers. A note is text of the same order as a saved routine, hundreds of bytes, and a note a model wrote is *preceded by the recorded model call that wrote it*, which is two orders of magnitude larger. Scratchpad volume is bounded above by a small fraction of a cost the log already pays, and the ratio is scale-invariant because both terms scale with turns. This resolves an [open question](confidence.md) toward the option the research lane was least sure of, for a reason the lane did not have.

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

### Directives are a kind of their own

The slot holds what the agent is. **Directives** are instructions about how to behave, and they are neither memory nor charter: they are scoped configuration with their own authors and their own lifecycle.

The live instance makes the distinction concrete. A connector mints a directive when it opens a context, saying what that context is like and how to behave in it, and it re-mints it whenever the context is re-established. Such a directive is per-context, not global; it is authored by a connector, not by the operator; and it is not part of who the agent is anywhere else. The self slot cannot absorb it, because the slot is always in context and singular, and the fact model cannot hold it either, because it has no teller, no truth value, and no audience.

A directive therefore carries four things:

| | |
|---|---|
| **Scope** | global, per-context, or per-conversation. Scope decides where it applies, and a directive is never in a prompt outside its scope |
| **Author** | operator or connector. Never the agent |
| **Lifecycle** | versioned like the slot, so re-establishing a context supersedes rather than appends. The observed failure is a directive re-appended verbatim ten times, which no amount of structural equality would have fixed, because each copy was a real event |
| **Composition** | narrower scope layers over wider, and a conflict is a teachable error to the author rather than a silent precedence rule |

**A connector-authored directive is bounded.** Nothing else in the design lets a connector write configuration, and the [connector contract](privacy-and-provenance.md) deliberately confines connectors to stubs and naming, so this is a new authority path and gets explicit limits: per-context scope only, never global, never able to touch the self slot, and attributed to the connector wherever it surfaces. A connector that is buggy or compromised can then shape one context's manner and nothing else, which is the blast radius the platform already has anyway.

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

### The path

`ingest` is a verb of its own and a job rather than a call. The agent supplies a source and the fields no extractor can infer, and receives a handle; the work runs on [the heartbeat's judgement budget](off-turn.md), not inside the turn. Five phases.

**The source layer lands first, and calls nothing.** The document is chunked deterministically, by heading and then by paragraph windows with fixed overlap, and every span commits as a gloss under one source memory, `observed` at the document's authored date and `recorded` now. This is why failure cannot lose the source: the source is durable before anything capable of failing has run, which is a construction rather than a handler.

**The gate is symbolic first and batched second.** A model call per span to decide whether a span deserves a model call is the trap this path exists to avoid. So a symbolic pre-filter drops spans with no anchor at all, no resolvable handle, no date, no quantity, no registered relation stem, which removes navigation, boilerplate, and front matter for nothing. What survives is digested to its heading path and opening sentence, and one batched call selects from the digest. The gate costs a call per few hundred spans.

**Extraction batches.** Selected spans pack into context-sized batches, one schema-constrained call each, every proposal tagged with its span. Cost becomes a function of document length over context window rather than of chunk count, which is the property that makes a long source affordable at all.

**Critics run per proposal and call nothing.** A rejection does not fail its batch: the span stays gloss-only under the mark [`off-turn.md`](off-turn.md) already defines, retried once. Per-span failure without per-span calls.

**Structure lands over the document.** Sections become memories under the source, linked by composition, with summarisation relations from a section to its digest, so a long work is navigable rather than one blob.

### Two things this has to argue rather than assert

**It is not deferred structuring.** The prohibition in [`write-surface.md`](write-surface.md) is on recovering structure from prose the store already treats as settled, which is the re-derivation tax. Here the source layer and its structure land in one job, and nothing reads the source as settled in between: a span committed in the first phase carries a pending mark that keeps it out of search and off every read surface until the job completes or degrades it to gloss-only. Ingest is one transaction spanning several blocks, which is the shape the write surface already accepts when it makes the returned parse a deferred handle.

**Audience is per document, not per Statement.** A per-Statement audience decision over several hundred Statements is not answerable by anyone, so the required-field discipline is met differently rather than waived: the document carries one transmission principle, inherited by every span and every Statement drawn from it, defaulting to the stricter of what the agent supplied and what the requesting conversation permits. Anything derived later takes the ordinary intersection. Where a document genuinely mixes audiences, the whole of it takes the strict principle and the agent re-records the public parts by hand, which is the same trade [an episode](two-traces.md) makes.

The cost model is four numbers, and stage 0c's harness produces them with the extractor pointed at a document instead of an entry: calls per hundred thousand words at a stated selection rate, recorded bytes per call, wall clock including the tail, and yield per selected span.
