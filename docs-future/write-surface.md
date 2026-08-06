# The write surface

The agent writes through two verbs. Everything else about the write path, extraction, critics, deduplication, and provenance, happens behind them.

## Recording what was said

The common case. The agent supplies an utterance and the fields no extractor can infer, and receives the parse:

```lua
local r = quill:record(
  "wren built quill in two days after rowan shared the architecture",
  { frame = "actual", visibility = "public" }
)
```

The return value is the point of the design. It carries what was committed:

| | |
|---|---|
| `r.gloss` | the utterance, stored once and shared by everything below |
| `r.event` | the Event, if one was extracted |
| `r.statements` | the typed claims, each addressable |
| `r.rejected` | what the critics refused, each with its reason |

Today a write is fire-and-forget, and whatever structure is eventually derived from it is derived by passes the agent never sees, long after it has forgotten what it meant. Here the parse comes back inside the same turn, while the context that produced it is still in hand, and it can be corrected:

```lua
r:amend(r.statements[3], { role = "source" })    -- wrong role
r:amend(r.statements[5], { frame = "persona" })  -- the character, not the bot
r:drop(r.statements[6])                          -- not something I meant to assert
```

An amendment is an ordinary write and passes the same critics. Dropping a Statement before the block commits removes the proposal; after commit it is a retraction like any other.

## Asserting a claim

When the agent knows exactly what it means, it says so, and nothing is extracted:

```lua
quill:claim("runs_on", "model/opus-4.8", {
  frame      = "actual",
  valid_from = "2026-07-16",
})
```

This path involves **no model call**. It is a direct structural write, checked by the critics and committed. It is what the agent reaches for when correcting itself, when recording its own observations, and when a conversation has already made the structure explicit.

## What the agent must supply

Three fields cannot be extracted, because they are judgements about the conversation rather than facts about the sentence.

**The frame.** Whether a claim is about an entity, the character it presents, the material that character draws from, or [the person behind the character](statements.md). A sentence about a persona agent's opinions is indistinguishable, on its own, from a sentence about the agent's configuration, and a detail about the human running the persona looks like both. Only the participant in the conversation knows which was meant, which is why the redirect is declared here and never inferred by an extractor.

**The audience.** Who may learn this. The utterance rarely says, and one utterance rarely has one answer.

A compound utterance cannot carry a single transmission principle, so the write does not ask it to. The call's `visibility` is a default for the parse, and the parse does not commit until every proposal's principle is settled: the agent either accepts the default for all of them or sets the ones that differ. Declining to look is not available, because the field is required per Statement rather than per call.

This is the [seam's own rule](the-seam.md) applied to the behaviour that carries the privacy commitment. Noticing that one clause of a sentence is sensitive is a judgement whose failure is silent and whose consequence is a leaked confidence, so it becomes a required field rather than a habit the prompt asks for. The corpus's flagship case is exactly this shape: one biography sentence yielding seven public claims and one that had to be held back, split by hand months later.

**The teller**, where it is not the obvious speaker. Relaying what someone else said is a different claim from saying it.

These are required fields, not optional ones, and a write cannot complete without them. Each has an explicit "not applicable" or "unknown" value, so declining is a recorded decision rather than an empty slot the model fills with something plausible.

## Where the structuring happens

Inside the write transaction. Not in the agent's head, and not in a maintenance pass afterwards.

`record` runs a schema-constrained model call at record time, which emits typed proposals, which meet the critics, which may reject them. This is the same treatment ordinary model calls receive: the call and its response are written to the log, and replay consumes the recorded response without calling anything.

The change against the current system is not who produces structure. It is when, how often, and whether it can fail:

| | today | here |
|---|---|---|
| when | later, in maintenance passes | at write time, in the transaction |
| how often | repeatedly, every pass, indefinitely | once |
| determinism | re-derived nondeterministically each time | recorded once, replayed from the log |
| visibility of failure | silent | a teachable error, in the same turn |

The last row is what the correction loop buys. A misparse today becomes a fact nobody notices until it is relayed back to someone months later. A misparse here comes back while the agent still knows what it meant.

## The cost, and what happens when it bites

Putting a model call on the write path is a real cost and a real risk, and the design should not pretend otherwise.

**It is not on the read path.** No read produces stored state from a model call. That is the fixed point, and nothing here touches it.

The fixed point is about derivation, not about latency. A read may consult a model for a **transient ranking input** that is never stored and never folded, which is the same exemption [`overview.md`](overview.md) already grants query embedding and which now also covers the reranking pass in [the query surface](query-surface.md). Anything a read computes this way is discarded when the read returns; nothing downstream may depend on it having happened.

**The failure mode to fear is thrashing.** A rejected write produces a teachable error, the agent retries, the retry runs another extraction, and the loop can spin. This is the existing teachable-error retry shape with a model call added to each iteration.

Four things bound it:

- **Extraction is per block, not per call.** Several `record` calls in one block structure together, so a block writing six facts pays once. The consequence is that `r` is a **deferred handle**: it is empty until the block ends, and the parse is read and amended in a later block of the same turn. The correction loop is same-turn, not same-block, which is what preserves the context that produced the write while still batching the extraction.
- **A structuring failure never loses the utterance.** If extraction fails or the critics reject everything, the gloss is committed alone, with no structure. The agent's words survive; only the structure is missing, and a later pass can supply it. A write path that can lose what someone said in order to protect its own schema has the priorities backwards.
- **Retries are capped.** A bounded number of structuring attempts per block, after which the write degrades to gloss-only and the agent is told so.
- **`claim` is always available and always cheap.** The escape from a fighting extractor is to say the structure directly.

**If it still thrashes, the dial exists.** Structuring can move to end-of-turn, or back to a pass, at the cost of losing the same-turn correction loop. The eager-against-lazy question is recorded as open in [`confidence.md`](confidence.md) precisely because it is an empirical call, and the constraint-tax measurement in [`evolution.md`](evolution.md) stage 2 is where it gets made. What must not happen is structuring-from-committed-prose becoming the **routine** path, because that reinstates the re-derivation tax the whole design exists to end. Structuring an utterance whose extraction failed is the exception the rule is drawn around: it is queued once, retried once, and either succeeds or stays gloss-only. The tax is a pass that re-reads everything indefinitely, not a queue that drains what fell through.

## Rejection is a teaching surface

A rejected proposal names what was wrong and what would have been right: which argument violated a declared range, which relation is deprecated in favour of which canonical form, which interval was malformed, which duplicate resolution was ambiguous and what would disambiguate it.

A persistent rejection that the agent cannot resolve is one of the four conditions that reach a person, because it usually indicates a schema gap rather than a mistake: a missing relation, a role the universal set lacks, or a frame the closed set does not cover.

## What is not on this surface

- **No direct Event construction.** Events arise from `record`, or from `claim` against an existing one. Hand-authoring role-edges is ontology-language, and the agent does not speak it.
- **No credence.** The agent cannot set how strongly a claim is believed. Credence is derived from evidence, and letting the writer assert it would reintroduce verbalised confidence through the back door.
- **No handle selection.** The agent writes to the handle it was given. Identity resolution happened before the turn started.
- **No critic bypass.** There is no force flag. An operator has a separate path; the agent does not.
- **No bulk ingestion.** A long document goes through [`ingest`](memory-typology.md), which is a job rather than a call: it batches extraction, carries one transmission principle for the whole source instead of a decision per Statement, and reports through a handle rather than a parse. Everything above describes the conversational path.
- **No charter writes.** The [self slot](memory-typology.md) is not reachable from the memory verbs. The agent can observe something about itself, which is an ordinary Statement, and it can propose a new version, which reaches a person.
