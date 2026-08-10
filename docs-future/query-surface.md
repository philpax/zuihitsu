# The query surface

The substrate is rich. The surface is not. Keeping the surface small is a design constraint rather than an oversight.

The agent addresses people and memories by handle, asks a small number of structured questions, and learns the edges of the API from errors at the point of failure. It never writes ontology-language, never names a frame it was not given, and never sees the machinery behind identity, credence, or audience resolution.

## Structural questions get structural answers

The change from today is that a question about structure is answered by traversing structure, rather than by searching prose and taking whatever the search returns.

- Who did this? Traverses the `agent` role-edges of an [Event](events-and-roles.md).
- When did this hold? Reads a [relation](relations.md) instance's validity interval.
- What happened between these two? Walks the events they both have roles in.
- How do you know? Reads the derivation record: the premises, the criterion, the tellers, and the assumptions in force.
- What changed? Compares validity windows across supersessions of the same claim.

Each of these is a deterministic graph query. None of them calls a model, and none of them depends on a similarity threshold.

"How do you know?" is worth singling out. It is currently unanswerable in any useful way, because judgement provenance records which model and template ran but not the evidence or the criterion. Making it answerable is what turns the store from something the agent trusts into something it can interrogate, and it is the same record that makes retraction propagate.

## Search as one signal among several

Semantic search is one signal among several rather than the ranking.

A result is ranked on similarity, on structural proximity to what is already in play, and on access recency and frequency. The latter two are independent of any embedding model and deterministic under replay, which matters because a similarity constant is a constant in one model's geometry and silently means something different after that model changes.

Access counts are foldable because an agent-visible read appends an event. The alternative, deriving them from state no fold reproduces, would break the first commitment for the sake of an event log line. The cost is small and stays small: a read event is a memory-id list against a log whose payload is dominated by recorded model calls two orders of magnitude larger, so reads add a fraction of a percent of bytes while raising the event count by roughly a tenth. The unit is deliberately the read the agent asked for, never the substrate lanes underneath it, so a fused multi-signal search stays one event however many rankings it merged.

The signals combine by fusing their rank orders, not their scores. Each signal produces a ranking, the rankings merge on rank position, and only the head of the merged list is worth a reranking pass. Fusing ranks is what keeps the combination embedder-independent: a rank order survives a change of embedding model, where a weighted sum of scores is a weighted sum in one model's geometry and means something different in the next. This is the convergent design in production retrieval, and it is the same argument the [drift](verified-write.md) section makes about calibrated thresholds, applied one level up from the threshold to the combination.

Every search result carries its episode anchor. A returned claim names the occasion it came from, so the agent can descend from the claim to the occasion to the verbatim turns without a second search. This is the retrieval side of [the two traces](two-traces.md), and it is why episodes are companions rather than a fallback tier.

## Reads resolve before the agent sees them

Three resolutions happen in the substrate, and the agent sees only the result.

Identity resolves to one handle per person. Sibling stubs, class membership, and merge credence are invisible.

Frame resolves to a default of `actual`, and a read must opt into `persona` or `source`. A question about what a bot runs on does not return what its character believes.

Audience resolves to the visible set, computed against who is present before anything is rendered. The agent is not handed content it must remember not to repeat, because withholding after the fact is exactly how residue leaks.

## Errors teach

A rejected write returns a teachable error naming what was wrong and what would be right: which argument violated a declared range, which relation has been deprecated in favour of which canonical form, which interval was malformed, which resolution was ambiguous and what would disambiguate it.

This is the existing pedagogy, with one change: the errors are now backed by sound checks rather than by convention, so the lesson is reliable. An error is the syllabus for a mistake rare enough that pre-teaching it in the prompt would cost more than it saves, which is the same reason the prompt stays small.

## What is not on the surface

Deliberate omissions, each because exposing it would push judgement onto the agent that the substrate should own:

- Class and merge internals. Nothing to second-guess.
- Numeric credence. A coarse ordinal with its evidence attached, never a number the agent cannot interrogate.
- Raw similarity scores. Ranking is the substrate's business; exposing the score invites threshold reasoning in the prompt.
- Assumption stamps and derivation internals, except through "how do you know?", which renders them as an account rather than as structure.
- The episodic wall's mechanics. An episode is marked as a reconstruction; the enforcement is not the agent's concern.

The test for anything proposed for this surface is whether the agent could make a wrong decision with it that the substrate would otherwise have made correctly. If so, it stays inside.
