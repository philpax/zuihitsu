# Data model

The data model below is a projection of the event log, not a set of mutable tables: every field is
materialised from events by the graph, and can be dropped and rebuilt from the log at any time (see
[Storage and materialisation](events-and-storage.md#storage-and-materialisation)). The pseudo-schema blocks name the fields as the projection exposes them (the graph's
`MemoryView`, `EntryView`, `RelationView`, `LinkView`); the wire form on the log is the corresponding
event payload.

## Memory

```
Memory {
  id:          ULID                  -- canonical, internal
  name:        string, unique        -- agent-facing handle
  description: string                -- synthesised prose, from PUBLIC entries only
  contents:    ordered list of ContentEntry   -- read separately, in commit order
  tags:        list<TagName>
  created_at:  timestamp
  volatility:  enum {Low, Medium, High}   -- modulates recency decay in search; defaults to Medium
}
```

Two-tier identity: internal references use the immutable ULID and agent-facing references use the
mutable `name`, so a memory can be renamed without breaking links. A rename records a
`MemoryRenamed` event and leaves the old name behind as an alias, so a renamed person is still
findable by a former name (see [Identity → Renaming](#renaming-the-same-memory-a-new-handle)); a current-name lookup always wins over an
alias. A memory is deleted softly (`MemoryDeleted` sets a flag): its contents stay in the log for
replay and audit, and live reads exclude it.

The projected `Memory` (the graph's `MemoryView`) carries `id`, `name`, `description`, `volatility`,
`created_at`, and its applied `tags`; its content entries are a separate read (`<memory>:entries()`),
ordered by commit order, not an inlined field. `volatility` defaults to `Medium` and is set by a
`MemoryVolatilitySet` event.

**One description, synthesised from public entries only.** A description is synthesised prose.
Synthesising it from a mix of public and private entries would put the regeneration model in the
position of compartmentalising across the visibility boundary at write time, and a leak there is
durable and broadcast — baked into state and surfaced to everyone, far worse than a transient
conversational slip. By building the description only from `Public` entries, the regeneration model
provably never crosses the boundary. `Attributed` entries — visible to anyone, but secondhand — are
also excluded from the description, so the prose never launders a relayed fact into an
unattributed-sounding statement. Per-audience precision in conversation comes from the
deterministically-filtered `recent_facts` in the brief (see [Contextual briefs](conversations-and-briefs.md#contextual-briefs)), not from the
prose.

The cost is that the summary of a person is blander than it could be, since it can't reflect private context — the right trade. The description has one scope; a relation-keyed scope (for instance, a fuller summary for trusted intimates of the subject) would reintroduce a write-time compartmentalisation boundary.

## ContentEntry

```
ContentEntry {
  id:            EntryId             -- stable ULID, globally unique; addressable for supersession, arbitration refs, per-entry vectors
  asserted_at:   timestamp           -- when the agent recorded it
  occurred_at:   Option<TemporalRef> -- what real-world time it's about
  text:          string
  told_by:       Teller               -- Participant(MemoryId) | Agent | Bootstrap; who told the agent this
  told_in:       Option<ConversationRef> -- provenance: the conversation + turn it was said in (not a visibility gate)
  visibility:    Visibility
  superseded_by: Option<EntryId>     -- the newer entry that replaced this one, once superseded; a retraction stamps the entry's own id here as a tombstone (no successor)
  retracted_reason: Option<string>   -- why the entry was retracted, once withdrawn; None for a live or plainly-superseded entry
  origin:        EntryOrigin          -- Recorded, or PlatformConnector(platform) when a connector owns the entry; derived from the recording event's source. A connector-owned entry is excluded from the maintenance passes (see below)
}

Visibility =
  | Public                           -- visible to anyone including the subject, distilled into descriptions
  | Attributed                       -- visible to anyone, but secondhand: carries a provenance
  |                                  --   marker and is never distilled into a description
  | PrivateToTeller                  -- surfaces only while the teller is present, and never to the memory's subject
  | Exclude(list of MemoryId)        -- as PrivateToTeller, additionally suppressed while any named party is present
```

The two times matter, and conflating them is a recurring source of bugs. "Marcus told me Monday he
visited Sydney last year" has `asserted_at = Monday`, `occurred_at = last year`. Search ranks it as a
"last year" memory by relevance; the brief's `recent_facts` treats it as a "Monday" entry by recency.
`asserted_at` is always set at write time; `occurred_at` is optional and may be vague — it is a typed
`TemporalRef` (an instant, a calendar day, a range, a fuzzy point, a recurrence, or a reference
anchored to another memory's occurrence; see [Time](time.md)), not a bare timestamp.

`occurred_at` may be authored at append time — the agent stamped it — or resolved after the fact by
the [turn-end temporal-extraction pass](write-path.md#coalesce-then-regenerate-once), which reads the entry's natural language ("last Tuesday") and
emits an `EntryTemporalResolved` event (or `EntryTemporalResolveFailed`, log-only, when it cannot
parse). The projection tracks which via an `occurred_authored` flag and denormalises a representative
sort instant (`occurred_sort`) for recency ranking; an authored occurrence is preferred over an
extracted one, so a guessed date never shadows a stated one. A seed entry that merely mirrors its
memory's description (the first entry `memory.create` appends from its `description` argument) is
marked `EntryDescriptionMirrored` and skipped by the extraction pass, since restating what a memory
*is* names no time.

`told_in` is provenance, not a visibility gate: it records the conversation reference (a specific turn or the room itself) the fact was said in, resolved at surface-build time to the room's display name and confidentiality, so a surviving non-public entry can carry a marker that names where it was said. (A conversation is a durable room; see [Conversations and briefs](conversations-and-briefs.md#conversations-and-contexts).)

Supersession is append-only: `MemorySuperseded` stamps the original entry's `superseded_by` and
leaves the original `MemoryContentAppended` immutable. Live surfaces then exclude the superseded
entry, while history surfaces (`<memory>:history()`, the console) still show it.

Consolidation is the many-to-one counterpart: `EntriesConsolidated` tombstones a cluster of
semantically-overlapping source entries (stamping each one's `superseded_by` = the replacement
entry id, exactly as supersession does) and records the full source list and the synthesized
replacement. A maintenance pass running under `Authority::Agent` (the maintenance-pass authority
tier — permits cross-teller supersede and free `same_as` assertion while blocking `self` writes)
synthesizes the replacement entry, which carries `Teller::Agent` and the least-restrictive
visibility of its sources. See [Maintenance passes](maintenance-passes.md).

Retraction is the tombstone counterpart, for when a fact has no in-place replacement — most often because it was filed on the wrong memory. `EntryRetracted` (via `<memory>:retract(entry, reason)`) records why the fact was withdrawn and drops the entry from every live surface exactly as supersession does: the projection stamps `superseded_by` with the entry's *own* id — a self-referential tombstone, so every `superseded_by IS NULL` live filter hides it with no successor to point at — and records the reason in `retracted_reason`, which is what tells a retraction apart from a supersession. The original `MemoryContentAppended` stays immutable, and history surfaces show the tombstone with its reason. A reason is required (an unexplained retraction is unauditable). There is deliberately no move affordance: because visibility resolves per memory (see [Visibility](visibility.md)), relocating an entry in place would silently rewrite its meaning, so the honest correction is to retract it here and re-assert it on the right memory with a fresh append (carrying the original `told_by`, and `occurred_at` when the date is known).

The visibility enum is deliberately small. `Public` is the default, so an entry recorded without a visibility replays as public. The enum has no explicit-allowlist lock-down variant; private group chats do not require one, and omitting it keeps the predicate small. The write-time *default* posture is computed,
not fixed at `Public`: a participant relaying something about *someone else* defaults
`PrivateToTeller`; self-disclosure and any non-person memory default `Public`; and agent-authored
content *about a person* has no default at all — it must classify itself before it is recorded, so a
re-recorded confidence can never silently default to public. The read-time predicate that interprets
these against the present set, and the provenance markers surviving non-public entries carry, are
described under [Visibility](visibility.md).

## Tag

```
Tag { name: unique string, description: one-line purpose }
```

A tag is a name plus a one-line purpose (`TagCreated`); creation always forces a purpose, while
application (`TagAppliedToMemory`) never mutates the description. The projected tag vocabulary also
carries a live-memory `count` per tag, surfaced in `tags.list` and the system prompt's tag-vocabulary
block. Changing a tag's purpose is a `TagDescriptionChanged` event.

## Link

A directed edge between two memories, instantiating a registered relation.

```
Link {
  from:       MemoryId
  to:         MemoryId
  relation:   RelationName
  source:     enum {Agent, Operator, Inferred}   -- who authored it
  told_by:    Option<Teller>  -- who asserted the relationship; the provenance a belief relation turns on
  told_in:    Option<ConversationRef>  -- the conversation + turn the link was asserted in
  visibility: Visibility       -- the audience posture, governing the read-time link_visible predicate
}
```

The materialiser canonicalises direction at write time, so `links.create(dave, "mentors", erin)` and
`links.create(erin, "mentored_by", dave)` produce the same edge. `source` records authorship: `Agent`
for the agent's own link, `Operator` for one asserted through the console — including the merging
`same_as`, the operator's alone to author (see [Cross-platform identity](#cross-platform-identity-operator-asserted)) — and `Inferred` for a link
the off-hot-path link-inference pass extracted from memory content (see [Write path → link inference](write-path.md#link-inference)).

Links carry the same visibility machinery as content entries. `told_by` is the teller who asserted the relationship — the provenance an asymmetric-belief relation turns on — and is `None` for a link with no teller behind it (the operator's merging `same_as`) or one recorded without a teller. The write-time `visibility` default is the same computed posture as an entry's (see [Visibility](visibility.md#defaults-at-write-time)), read off the link's endpoints: a relayed fact where the teller is neither endpoint defaults `Attributed`, and a belief where the teller is one endpoint about a different-person target defaults `PrivateToTeller`. The read-time `link_visible` predicate guards a directed link by its target and a symmetric link by both endpoints.

## LinkRelation (the registry)

```
LinkRelation {
  name:        string          -- canonical
  inverse:     string          -- may equal name for symmetric
  from_card:   One | Many
  to_card:     One | Many
  symmetric:   bool
  reflexive:   bool
  description: string          -- one-line purpose, surfaced in the prompt and links.list/get
}
```

One relation, two labels; cardinality declared once and the inverse view computed. Registered via
`links.register` and recorded as `LinkTypeRegistered` events. A relation's `description` is its
one-line purpose, surfaced in the system prompt's relation registry and in `links.list`/`get` so the
agent knows which relation fits which situation. The registry lives in data, not code: the seed
ontology (see [The seed ontology](../CONTRIBUTING.md#the-seed-ontology) in `CONTRIBUTING.md`) is the minimum-viable set genesis registers, and the agent coins
the rest at runtime.

## Naming conventions

Memory names use namespace prefixes. The recognised namespaces are exactly:

- `person/<handle>` — people
- `place/<handle>` — places
- `event/<handle>` — things that happen at a time (appointments, meetings, recurring schedules)
- `topic/<handle>` — subjects of interest
- `context/<handle>` — conversations (a channel, DM, or group chat; one per durable conversation, minted eagerly alongside the room)

`self` is the reserved handle for the agent's own self-model. It is not a namespace — it sits in no
namespace and gets no subject-guard — but it is the one other reserved name the scaffold teaches.

The recognised set lives in one place in the code (the `Namespace` enum, plus the reserved `self`
handle), so the scaffold that teaches the prefixes and the code that mints and reads handles cannot
drift. A memory name is otherwise a free string: `memory.create` accepts any name and does not reject
an unrecognised prefix or a bare name, so a handle like `project/atlas` is a perfectly valid memory —
it carries no *kind*, since only the five prefixes above are recognised. In particular, only
`person/` memories carry a subject-guard, so a non-person memory's private entries have no read-time
subject protection (see [Visibility](visibility.md)).

Prefixes make a memory's kind visible at a glance, make prefix-scoped queries cheap
(`memory.search("person/")`), and make cross-category collisions structurally impossible:
`place/sydney` and `person/sydney` are simply different memories. The namespace is what kind of thing
a memory is; tags are what it's about. Disambiguating suffixes are encouraged within a namespace
(`person/dave-chen`, `person/dave-patel`). Names are unique: creating a second memory over an existing
name is a teachable error that points the agent to `memory.get` (or `memory.get_or_create` when
existence is uncertain) rather than minting a duplicate — the fail-on-exists strictness is
load-bearing for the merge and identity flows. The collision error also lists the near-matching
existing handles in the same namespace, closest first (`person/dave-chen`, `person/dave-patel` when a
create for `person/dave` collides), so the agent picks a distinguishing name for a genuinely different
subject rather than colliding again or minting a near-duplicate. Likewise `tags.create` raises on an
existing tag name, listing the near-matching tags and pointing the agent to apply one or change its
purpose instead.

An event that recurs is held as ONE memory under a generic name (`event/book_club`), with each
occurrence dated on its own entries — never a date-stamped clone per mention (`event/book-club-july`).

## Identity and participants

### Platform-ID mapping

Platform-level participant IDs map to `person/*` stubs through an operational lookup table keyed `(platform, platform_user_id) → memory_id`.

The mapping is seeded by `ParticipantIdentified` events, so it lives in the log and rebuilds with every other projection, but it is materialised as a table separate from the memory graph's nodes and edges, because these are operational identifiers, not facts about people.

`platform` is a short stable key from operational config (`direct`, `discord`, `slack`, and so on, with `direct` reserved for the operator's own console), and a `ParticipantIdentified` binds one stub to one `(platform, platform_user_id)` pair. The binding is emitted both on first contact (paired with the `MemoryCreated` that mints the stub) and whenever an existing stub later gains a further platform identity, so one memory can carry several platform bindings.

The agent-facing way to name a specific stub is its handle, and a platform-qualified handle (`person/dave@discord`) is simply that stub's name — `memory.get` resolves a name to exactly one memory (`memory_by_name`), never to a class. The `@platform` suffix is not parsed at resolution time; it is just part of the name every platform-arriving stub is minted under (see below).

### Stub creation on first contact

The first time the agent encounters someone on a platform, it eagerly mints a `person/*` stub for them: a `MemoryCreated` for an empty memory plus the `ParticipantIdentified` that binds it to the `(platform, platform_user_id)` key. An unused stub costs almost nothing; not having a node to attach a fact to mid-conversation costs a tool call at the worst moment.

The stub is named by the participant's **platform-qualified handle** — `person/<platform_user_id>@<platform>` (so a Discord user 12345 mints `person/12345@discord`), which the agent later humanises with a rename (`person/dave`). Qualifying by platform is what keeps two people who happen to share a handle apart: Discord user 12345 and Slack user 12345 mint `person/12345@discord` and `person/12345@slack`, two distinct nodes that never collide. The `(platform, platform_user_id)` binding lives in `ParticipantIdentified` independent of the name, so the name is free to be renamed later without breaking resolution.

The resolver (`resolve_or_mint_participant`) mints along one of two paths, keyed on the qualified name:

- **The key is already bound.** A prior `ParticipantIdentified` already binds this `(platform, platform_user_id)` to a memory — the resolver returns that memory and mints nothing.
- **The qualified name already exists as a memory** — an agent-authored stub the agent named under this exact `@platform` handle before the participant ever spoke. The resolver **binds the platform identity to it** (a `ParticipantIdentified` alone, no second node), so the arrival and the stub are one memory from the first turn rather than two the agent must later reconcile. This is safe because the agent authors hearsay under human-readable handles (`person/dave`), not platform-qualified ones (`person/12345@discord`): a collision requires the agent to have named a stub under the exact platform-qualified handle, which is a deliberate act, not an accident. A wrong binding is still reversible by an operator `unlink`.
- **Otherwise**, the resolver mints fresh — `MemoryCreated` plus `ParticipantIdentified` — the ordinary first-contact case.

### Renaming: the same memory, a new handle

A memory's handle is a mutable label over an immutable ULID, and every relational reference — links, content entries, `told_by`, the `(platform, platform_user_id)` binding, `same_as` membership — is keyed by the ULID, never the handle (the two-tier identity of the **Memory** model: immutable id, mutable name). So renaming is safe by construction: `MemoryRenamed { id, old_name, new_name }` updates only the `name` column and its FTS row, and the memory carries its whole history forward under the new handle as one continuous node.

That is what lets the agent accommodate a person changing the name they go by — a transition above all, but equally a married or a chosen name — with no loss and no confusion. When someone asks to be called something new, the agent **renames their existing memory** (`<memory>:rename("person/sarah")`); it does not create a fresh one. The distinction is the whole point: a rename keeps the single identity — the agent reads the same facts, links, and confidences under the new handle — whereas a new `person/sarah` would split the person across two unlinked nodes the agent cannot reconcile (it cannot assert `same_as` itself; see below). The scaffold steers hard toward rename for exactly this case, because the failure mode is not data loss (the ULID is safe either way) but the agent fragmenting or misaddressing a person it already knows.

The old name is **held for resolution and recognition, not for display** — the distinction that keeps deadname-safety and bridging from fighting. `MemoryRenamed` records `old_name`, and the materialiser keeps it as an **alias** of the renamed memory (a `former_name → memory_id` row in `memory_aliases`, last-writer-wins on the rare collision where two memories shed the same name). This does two things, both at the *read* surface rather than through prompt instruction.

First, an old name still *finds* the person — by exact handle and by search alike. `memory.get` resolves it and returns the handle **flagged as a former-name match** (a `former_handle` field, under the memory's current `name`); and `memory.search` finds them too, because the vacated name is folded into the renamed memory's FTS content (ranking only, never displayed), so an old-name search surfaces the person even when their current content never uses it.

Second, every read of a renamed memory carries its prior handles so the agent connects its older, old-name *content* to the same person: a search hit reads with a `[formerly person/dave]` marker (the same marker family as `disputed`/`stale`), and the `memory.get` handle exposes a `former_names` list (also rendered as a `formerly …` line in the `mem:details()` header). So the agent learns *at the point of reading*, rather than from a rule it has to recall, that `person/sarah`'s history written under "Dave" is hers — which is where the confusion otherwise arises: it reads "I'm Dave" inside Sarah's entries and splits her in two.

These fields are passive, though, and a small model under load skips them: it looks the person up under *both* the old and new handle, reads each one's raw entry text, and — seeing identical content on what it took to be two memories — concludes there are two people. So an old-name `memory.get` also emits an **active note** into the agent's own output, the stream its `print` feeds back to it: `note: "person/dave" now goes by "person/sarah" — the same person, renamed.` It rides the agent's result regardless of how it goes on to inspect the handle, fires only on the rare old-name lookup, and — like every other former-name surface — reaches only the agent, never a participant, so it stays deadname-safe.

But the old name never *surfaces on its own*: a current name always wins resolution (the alias only fires when an old name is already in play because a speaker invoked it), the former-name marker rides only the agent's own reads — never the description or the brief that reach participants — and the agent answers under the current name.

**A rename re-synthesises the description** even though it changed no content: applying `MemoryRenamed` advances the memory's `last_content_seq` (the same watermark a content append moves), so a renamed memory reads as stale and the describer re-composes its always-visible summary under the new handle. Without this the description — which *does* reach participants in a brief — would keep the old name indefinitely, the one place the deadname would otherwise broadcast. Recognition without broadcast.

The honest limit is that content entries are immutable: historical prose written under the old name still contains it verbatim. The system surfaces the rename context alongside that prose, and refreshes the synthesised description, rather than rewriting what was already said.

Renaming is **guarded, not gated**: unlike a `same_as` merge, it creates no cross-context surfacing — it is the same node throughout — so the agent renames within an identity freely, subject only to three guards. It cannot rename `self` (a platform-authority write to `self` is refused; `self` is operator-only); it cannot rename onto a handle that already belongs to a *different* memory — that is a collision, a teachable error (`NameExists`), never a silent merge of the two (renaming a memory to a name it already holds is a no-op); and it stays out of the platform-qualified namespace in both directions (below). Reconciling two genuinely separate stubs remains the operator-asserted `same_as` path below, never a rename onto an occupied name.

**The connector rename contract.** The platform-qualified namespace (`person/<user>@<platform>`) belongs to the connectors, and agent renames are refused in both directions (teachable errors; operator authority is exempt). Moving a stub's name away would desynchronise it from the platform's own view: the binding that routes messages is the stable `(platform, platform_user_id)` key, never the name, so a rename cannot misroute — but the qualified handle exists to *mirror* the platform, and only the connector's side knows when that changes. Renaming another memory *onto* the qualified shape is the sharper hazard: a first contact binds a platform identity to whatever memory already bears the qualified name, so claiming the shape would squat a future participant's binding. The obligation this places on connectors splits by how they key participants. A connector whose `platform_user_id` is a mutable handle owns keeping the stub's name in step with the platform when the platform-side name changes. A connector keyed by stable opaque ids (the Discord connector's snowflakes) mints stubs whose names never need to change; the readable name rides as identified attributes, and canonical naming is delivered by a bare `person/<name>` profile merged onto the stub — the bare namespace the rename guards deliberately leave agent-writable.

### Cross-platform identity: operator-asserted

A single human may appear as several stubs — you on the direct interface, you on Discord. A `same_as` link reconciles them into one identity, and it reaches the log by exactly one path: an **operator assertion** through the console. The operator knows the truth and states it, which authors `LinkCreated { relation: "same_as", source: Operator }` under operator authority — the only authority that may author a `same_as` at all. What never happens is the agent merging two identities *directly*. A turn's `link("same_as")` is not honoured as a merge: for a platform-authority block a create is re-routed into an inert `MergeProposed`, leaving the block's other writes intact, and a retraction of an existing `same_as` is rejected outright (`MergeForbidden`). The agent guesses, but it does not get to act on the guess unaided.

Why the gate sits *here specifically*: a `same_as` merge enables cross-context surfacing — a confidence told on one platform can reach the merged identity on another — so a wrong or socially-engineered merge is a leak. It is the one genuinely sensitive identity operation, and everything else about a person the agent already does autonomously (their single within-platform handle, renaming it, recording and reading their facts). The agent may still raise the question — it proposes a merge and the operator decides — but the crossing itself waits on operator judgement, never the agent's say-so and never the conversation's (see [Trust and authority](trust-and-authority.md#trust-model) for why no automatic path is safe).

The flow is **propose → operator confirms**:

- `<memory>:propose_merge(other)`, optionally carrying a stated rationale, records a `MergeProposed` — the agent's judgement that two stubs may be one human. It is **inert**: not a `same_as`, not projected into the graph, so both stubs stay in their own classes and *nothing surfaces across the would-be merge*. A proposal is a recorded belief, not a merge. Its `source` records who raised it — always `Agent`, a `propose_merge` from a turn — and a proposal naming one memory twice is rejected as a teachable error (`MergeProposalInvalid`). Its optional `rationale` is the proposer's stated grounds, which the operator weighs as a claim, not as evidence.
- A proposal **pends until the operator confirms it**. There is no automatic crossing: every proposed merge waits for the operator, and confirmation is the only thing that authors the merging `same_as`. So a proposal changes nothing about what surfaces until the operator acts on it — both stubs stay distinct, and the "might be the same person" belief lives only as the recorded proposal.

Pending proposals surface in the console, not only in the log: the operator sees each still-unmerged pair and confirms it directly through a control endpoint (`confirm_merge`), authoring the merging `same_as` (`source: Operator`). There is no decline counterpart — a proposal has no recorded settlement of its own, so one the operator is not convinced by simply stays pending. The operator is not limited to the agent's guesses: the same control also asserts a merge the operator knows to be true independently of any proposal. A pair whose two stubs already share a class has been merged and drops off the list — there is nothing left to decide.

`same_as` is symmetric, and its equivalence classes are transitively closed at materialisation time via union-find, producing a denormalised `class_id` on each memory in the projection. Membership tests, presence checks, and lock acquisition then reduce to an indexed equality on `class_id`. The recompute runs on every `same_as` link change — a `LinkCreated` unions two classes and a `LinkRemoved` re-splits the affected component — as a whole recompute rather than a local patch (trivial at personal-agent class sizes). Because a `same_as` edge is only ever operator-asserted — never raw agent inference — classes stay small and trustworthy.

### Read-time traversal

Agent-facing reads — `memory.get`, search, and the traversal methods — surface content and links from the entire `same_as` class of the queried memory (every class-traversing read keys on the shared `class_id`), so the agent treats you-on-Discord and you-on-direct-interface as one continuous identity without chasing the relation by hand. Each entry lives on exactly one member, so a class read gathers each fact once; links internal to the class (the `same_as` plumbing and any within-identity edge) are dropped, since a relationship the agent reasons about points out of the identity.

Per-stub provenance is preserved: each entry's `told_by` and each link's endpoints retain their original stub references, so the agent can still distinguish "said on Discord" from "said on the direct interface" when it matters.

### Writes target a stub; reads traverse the class

Writes are never fanned across the class: an append lands on exactly one stub, and `memory.get` looks a handle up by name and returns that one memory, never a synthetic class object. Which stub the write lands on turns on the handle. A **platform-qualified** handle names one binding — `dave@discord:append(...)` writes the Discord stub — so a fact deliberately scoped to a platform stays there. A **platform-agnostic** handle (`person/dave`) addresses the merged identity as a whole, so a class-level fact recorded through it is redirected to the class's **primary stub** rather than attaching to whichever member the unqualified name happens to resolve to. The redirect keys on the addressed stub's own name: a handle carrying no `@<platform>` suffix is class-spanning and widens to the primary; a qualified one writes its exact stub.

The class's distinguished member, the **primary stub**, is what that redirect resolves to: the denormalised `class_id` the union-find recompute stamps on every member is the class's **earliest member by ULID** (ULIDs sort chronologically, so the primary is the oldest stub) — unless the operator has designated one. The primary is also what class membership, presence checks, and lock acquisition reduce to — an indexed equality on `class_id` — and it is the anchor the whole-class reads and synthesis fold over. The choice of primary is deterministic and merge-order-independent (a merge of two classes takes the earliest ULID across the union), which keeps `class_id`-keyed reads and the write redirect stable regardless of how the class was assembled. The redirect reads the committed `class_id`, so it is a pure function of the log and replays identically; a stub created in the same block has not yet joined a class, so a write to it is never redirected.

The earliest-ULID default loses to a throwaway stub the agent minted before it learnt the operator's canonical handle: the throwaway is older, so it wins the class by age. The operator can override the default from the console's Relations view, which records a `ClassPrimaryDesignated` on the chosen stub; the recompute then resolves the class through the earliest-ULID *designated* member instead. The designation lives on the memory, so it survives the stub's later unmerge into another class (it simply governs whichever component the stub then belongs to), and a designation naming a stub outside a component has no bearing on that component's primary. Two designations in one class fall back to the earliest-ULID designated stub, keeping the choice merge-order-independent like the default.

So a class-level human-fact — a third-party aside about the person that belongs to no particular platform, like Erin telling the agent something about Marcus in a DM — lands on the primary *by design*. Recording it under `memory.get("person/marcus")` widens to the primary even after the operator designates a stub other than the one the unqualified name resolves to. This is the case the redirect exists for: without it, the fact would land on the name's exact stub and quietly diverge from the primary the class is meant to cohere around. The one exception is the operator's own anchor: `person/operator` is the earliest-ULID primary of the operator's class yet holds no content of its own (see [Self-merge and the operator's continuity](#self-merge-and-the-operators-continuity)), so a write on the operator's real `person/<name>` profile is *not* redirected onto the anchor — it stays on the profile where operator facts belong.

Because synthesis traverses the whole class (see [Visibility](visibility.md)), the fact surfaces for the entire class regardless of which member holds it — so the redirect is about *where a fact is anchored*, not whether it is found. The disambiguation the qualified handle affords is reserved for the genuinely stub-specific case: attributing to one platform ("Dave said *on Slack*…"), which the agent expresses by naming the exact stub with `memory.get("person/dave@slack")`. Only content writes redirect — an append, a supersede or revise, and a memory's volatility classification; a rename acts on the handle itself and a tag or a link names its exact endpoints, so none of those is widened.

### Self-merge and the operator's continuity

The operator has a reserved identity anchor, `person/operator`, minted on first imprint. It carries **no content of its own** — a write to it is refused (`person/operator is a provisional anchor`) — because facts about the operator belong on their real `person/<name>` profile, which is merged into the anchor with `same_as`. The anchor stays a pure merge target so the operator has one stable handle across whatever real profile the imprint later resolves to.

When the operator wants the agent to recognise them across the direct interface and Discord, they assert the `same_as` link through the console's merge control (operator authority — the only authority that may author `same_as` directly). From that point the agent reads the operator as one identity across both surfaces. See [Synthesis traverses the `same_as` class](visibility.md#synthesis-traverses-the-same_as-class) for how this affects descriptions.
