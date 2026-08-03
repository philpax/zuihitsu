# Grounding brief: zuihitsu ontology redesign research

Shared context for all research lanes. Read this before doing anything else.

## What zuihitsu is

A neurosymbolic personal-agent harness. The symbolic half: an append-only event log (sole source of truth, deterministic replay; model and embedder calls happen at record time only, never at replay) materialised into a knowledge graph (SQLite). The neural half: an LLM agent living in conversation, acting through a sandboxed Luau scripting API. One instance = one agent = one log. The agent meets people across platforms (Discord, direct console), remembers what each said, and keeps confidences between them.

## Current ontology (the thing being redesigned)

- **Memory**: a named node (`person/dave`, `place/sydney`, `event/book_club`, `topic/x`, `context/room`, reserved `self`; free-form names allowed but kindless). Two-tier identity: immutable ULID + mutable handle; renames leave aliases.
- **ContentEntry**: a prose sentence attached to exactly one memory. Fields: `asserted_at`, optional typed `occurred_at` (`TemporalRef`: Instant | Day | Range | Approx{center,fuzz_days} | Recurring(rrule) | BeforeAfter{dir,anchor-memory}), `told_by` (Participant | Agent | Bootstrap), `told_in` (conversation provenance), `visibility`, supersession/retraction tombstones.
- **Visibility postures**: `Public` (distilled into descriptions), `Attributed` (visible to all, carries `[via X]` marker, never distilled), `PrivateToTeller` (surfaces only while teller present, never to the memory's subject — the subject-guard), `Exclude(set)` (read-only variant). Deterministic `visible(entry, present_set)` predicate applied at every surface (brief, search, direct reads, links). Fail-closed defaults. Contextual-integrity framing is explicit in the docs.
- **Attestation**: an entry is a fact a set of tellers stand behind; each attestation carries its own posture; audience-widening invariant (no attestation wider than founding posture); hidden attestations leave zero residue on agent-facing surfaces; per-attester retraction with last-attestation death.
- **Links**: bare directed edges `{from, to, relation, source, told_by, told_in, visibility}` — no attributes, no validity interval. Relation registry (`LinkRelation {name, inverse, cardinality, symmetric, reflexive, description}`) is agent-coined at runtime, event-sourced, but immutable once registered (#42). Seed ontology is a minimum-viable structural set (identity, participation, composition, placement, origin, operatorship, acquaintance).
- **Identity**: per-platform stubs (`person/12345@discord`), `same_as` equivalence classes via operator-confirmed merges only (agent may only propose); union-find `class_id`; reads traverse the class, writes land on one stub (class-level writes redirect to a primary stub). Renames are free; merges are gated because they enable cross-context surfacing.
- **Descriptions**: per-memory synthesised prose regenerated off the hot path from Public entries only (never Attributed/private) — the write-time compartmentalisation guarantee.
- **Maintenance passes** (`Authority::Agent`): dedup (cosine 0.95), consolidation (two-tier: within-visibility-level synthesis at 0.85; cross-level absorb-and-attest dedup), canonical profiles, link-redundant-entry cleanup. All re-derive structure from prose via embedding geometry + model judgement.
- **Time**: bitemporal asserted/occurred split; calendar is a view over memories with future `occurred_at`; wake-up scheduler fires on due occurrences; typed date objects (`calendar.today()`, etc.) so the model names operations rather than computing dates.
- **Agent surface**: handle-shaped and simple — Luau API (`memory.get/create/search`, `mem:append/supersede/retract/attest`, `links.create`, `calendar.*`), teachable errors as pedagogy, scaffold (system-prompt principles) + API reference. The agent never speaks ontology-language.
- **Authority tiers**: Platform (conversation turns; cannot write `self`, cannot assert `same_as`), Agent (maintenance passes), Operator (console; the only `same_as` author).

## The recorded failure classes (docs/ontology-failures/2026-07-23.md) — the redesign must address ALL of these

1. **Facts are sentences** — root failure. Entries are prose blobs; every downstream mechanism (dedup, consolidation, arbitration, temporal placement) re-derives structure from prose, repeatedly, nondeterministically, fallibly. Structural questions have no structural answer.
2. **One event, one subject, many copies** — an event with several participants shatters into per-subject re-phrasings (one implementation-event recorded 3x in one session). Motivates neo-Davidsonian one-event-many-roles.
3. **Relations are bare edges** — no attributes, no validity intervals ("worked at X 2019-2021" cannot be a relation); agent-coined vocabulary drifts (same relation coined 4 ways).
4. **Schedule/description conflation** — a single occurrence field means both "when the referent happens" and "when the agent should act"; `FREQ=DAILY` stamped on a fact *describing another bot's cron job* made the agent wake for it.
5. **Identity is binary and entangled with storage** — merge-or-stranger; no partial/context-dependent identity, no credence on merges; identity resolution entangled with fact storage. #94: knowledge-overlap adjudication unsound in principle (knowledge can be recited); direction = revocable composite identity + assumption-stamped derivations (ATMS-style).
6. **Relation schemas immutable** (#42) — mis-registered relation can only be abandoned.
7. **Identity complexity leaks into behaviour** (#104) — post-merge, agent failed to relay a sibling stub's history 7/10 runs; agent minted duplicate bare handles; distrusted its own fresh brief.
8. **Belief has no credence model** — arbitration is episodic prose notes; contradictions persist as flags; nothing represents strength of belief.
9. **Hygiene thresholds are embedder geometry** — cosine constants (0.95/0.85) in one embedder's space; embedder change silently invalidates all.
10. **Load-bearing behaviour is prompt-sensitive** — a capture behaviour moved ~6% → 75% on one scaffold sentence. The welding failure in miniature.
11. **The neural writer is unverified** — the model is sole writer of structure; guards check authority/visibility, never truth; judgement provenance records model+template but not evidence or criteria.

## Fixed points (not up for redesign)

- Append-only log as sole truth, deterministic replay.
- Privacy at least as strong as current: per-fact audience postures, hidden-endorsement semantics, zero residue of hidden knowledge on uncleared surfaces.
- Teachable-error pedagogy; agent-facing surface stays handle-shaped and simple (ontology may be arbitrarily rich underneath).
- Scale deliberately unbounded: bulk ingestion of long documents into structured navigable memory clusters (#44) must be handled, not excepted.
- No migration constraint: the proposal targets a future agent's genesis.

## Relevant open issues (summaries)

- **#94** identity: fact-overlap adjudication unsound (recitation attack); collective entity resolution on relational structure as strongest passive signal; tiered reversibility (revocable composite merge + assumption-stamped derivations; irrevocable effects gated on challenge-response); ATMS as research home.
- **#15/#20** self-model governance & autonomy: agent currently has NO path to write `self` (operator-only); `Agent` authority variant proposed; reflection/heartbeat design open; zero-administration endgame — operator intervenes only on exceptions.
- **#42** relation schema evolution missing.
- **#44** long-document ingestion into part_of/summarizes clusters; scoped memory IDs open question.
- **#58** procedural memories (saved Luau functions) — nowhere principled to live.
- **#59** transient private scratchpad — nowhere principled to live.
- **#74** episodic conversation search fallback — nowhere principled to live.
- **#100** in-block LLM calls: neural judgement inside symbolic transactions (retry/idempotency tension: an LLM call latches the no-retry flag).
- **#103** typed dates/durations across the whole agent surface.
- **#104** merged-identity behaviour failures (above).

## Deliverable discipline for lanes

Write your findings as a markdown file at the path given in your task prompt. Cite every load-bearing claim with a URL or a precise bibliographic reference. Flag uncertainty explicitly — a later adversarial-verification pass will check claims, and an overclaimed finding is worse than a hedged one. End with a section "Implications for zuihitsu" mapping your findings onto the failure classes and issues above, concretely.
