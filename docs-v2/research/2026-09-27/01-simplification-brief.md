# docs-future simplification brief

This brief governs a rewrite of `docs-future/` in the zuihitsu repository. It records decisions the operator has already approved. Treat every decision below as binding. Where the brief is silent, preserve the original's content in compressed form rather than dropping it.

## Why this rewrite exists

`docs-future/` specifies a successor architecture for zuihitsu that does not exist yet. The core ideas are sound and grounded in observed failures of the current system. The tree, however, grew from about 237KB to about 699KB between 10 August and 5 September 2026 through repeated review rounds that added precision and never removed any. It now carries machinery that the deployment does not need (one operator, one agent, a handful of participants, a server that is not publicly exposed), and it spends wire-level precision (CBOR layouts, canonical JSON fixtures with digests) on state it declares disposable. The live instance it was designed against holds 198 content entries from 3 human tellers.

The goal is a spec that keeps the representational fidelity and drops redundant machinery. The dominant source of redundancy is provenance stored on every object when it can be derived from the event log.

## Context that changes the constraints

- The current agents are frozen until the successor comes online. They are never migrated. Backward compatibility with the current system is not a concern.
- Before the successor's first real genesis, everything is disposable. Pre-genesis stages need no compatibility, rollback, or experimental-state disposition sections.
- After genesis, the operator does not want an ontology-wide migration or an agent reset. Additive changes and small recorded upcasts are acceptable.

## The keep-at-genesis test

A field or record kind is required at genesis only if it is part of an identity key, or if its value cannot be recovered later from the retained raw input. Everything else can be added later additively or by a recorded upcast, and the spec lists it as deferred instead of reserving it. Frame, polarity, and modality pass the test because they are Proposition identity coordinates and cannot be reconstructed from old structure. Page and text-span selectors pass because grounding a read book at page level cannot be recovered without re-reading it.

## Permanence contract (overview.md, README.md)

- Pre-genesis: disposable, as now, stated briefly.
- Post-genesis: persisted meaning and stable identity never change incompatibly. New capability is additive (new event variants, registered definition versions, new projections) or arrives through a recorded upcast.
- The upcast rule: an upcast may restructure data but never supplies a value that was absent from its input. A missing value becomes an explicit `unknown`.
- No change may require resetting an agent born on the successor.
- Distributed operation is a non-goal that the design must not preclude. Stable identities are ULIDs. Transitions name their predecessor (see lifecycle mechanics). Domain semantics never depend on the local log sequence number: the spec's `source_head` becomes an opaque "frontier" (a local sequence today, possibly a vector clock later). The spec does not design a sync layer. It notes that lifecycle transitions, Trigger firing, identity acceptance, and erasure are the parts that would need coordination, while Occasions, Assertions, and Attestations are append-only sets that merge naturally.

## Object model (statements.md owns it)

1. Occasion: an external input event. It owns one ordered sequence of content parts (text parts and ArtefactReference parts, interleaved; either may be absent), a `caption_of` marker on text parts, participants with witness evidence, and observed and recorded time. Lifecycle: live, invalidated (malformed or unauthorised source), erased.
2. Activity: any agent, operator, tool, or model action. Activities are the existing recorded events (model calls, Lua blocks, tool calls, operator actions), not a new object kind with its own lifecycle. Every model-call Activity records a context manifest.
3. Context manifest: for each model call, the ordered list of stable IDs of every object whose content was rendered into that call's context: memory reads, brief components, conversation-history Occasions, tool outputs, Perceptions, working notes, generated episodes, and so on. Content without an object ID (prompt templates, the API reference) is identified by its template name and version. Every renderer goes through one choke point that appends to the manifest. Influence, taint, non-evidentiary marks, restriction intersection, and access accounting are projections over manifests plus Activity input and output edges. They are not stored on objects. The spec must state that manifest completeness is the load-bearing invariant and that a missing entry under-taints silently.
4. Proposition: a computed canonical key over (subject, relation, object, frame, polarity, modality), not a stored record. Keep the object variants, frames (`system`, `persona`, `source`), the `principal` write-time redirect through `presents`, polarity canonicalisation of lexical negation, and the modality set (`actual`, `planned`, `hypothetical`, `habitual`, `deontic`, `cancelled`) with its rules.
5. Assertion: a Proposition situated in typed validity, with immutable mode `asserted` or `quoted`. Lifecycle: candidate, settled (and demotion back), validity closure with a cause, superseded, retracted, invalidated, erased.
6. Attestation: one source's support for one Assertion. It carries a typed source, expressed in the spec as a tagged union that the implementation encodes as a Rust enum:
   - `testimony`: teller, Occasion, source locators, expression strength (`hedged`, `plain`, `emphatic`), transmission principle, witness and dependence lineage;
   - `observation`: an agent, operator, or tool Activity and its observation edges;
   - `derivation`: the producing Activity, ordered typed inputs, criterion and implementation versions, ontology and policy versions, the ResolutionEnvironment, assumptions, and frontier.
   Only `testimony` contributes teller support or corroboration. This merges the original Attestation, SourceAuthority, and assertion-producing Derivation into one record. The standalone Derivation object is removed. Perceptions and derived Artefacts record their producing Activity and typed inputs directly.
   Attestation lifecycle: live, superseded, retracted, invalidated, erased. Losing the last live testimonial support changes the support projection, never the Assertion's identity.
7. Perception: the fallible output of a model or tool Activity over an ArtefactReference and selector. It is never testimony. Lifecycle: current, superseded, retracted, invalidated, erased.
8. Artefact: a minted ULID for one immutable byte sequence. The content digest is stored in the erasable payload alongside the byte metadata, and the deduplication index is built from payloads, so an erased Artefact's tombstone keeps only its ID. Do not use a content hash as the ID: an erased file's tombstone would then let anyone confirm a guessed file. Drop algorithm-independent digest assertions, digest rotation, and collision quarantine. Availability folds to available, unavailable, or erased. Retention is the set of live authorised references.
9. ArtefactReference: unchanged in substance. One sharing act on one Occasion, with supplier, filename, media type, part position, and transmission principle. Lifecycle: authorised, withdrawn or retracted (reversible), erased (terminal).
10. Selectors: a content-keyed value with no minted ID, whose equality is byte equality of a canonical encoding. Leave the encoding to the implementation; drop the CBOR key layout. Genesis variants:
    - `whole_artefact`;
    - `page_range`: half-open, zero-based, under a named and versioned document decoder;
    - `text_span`: a half-open range of Unicode scalar offsets over a derived text Artefact. Text extraction produces that derived Artefact once, together with a page map back to the original, so a claim from a read book can cite a span and resolve its page.
    `spatial_region`, `frame_range`, `time_range`, and `byte_range` are deferred (additive).
11. Event: a minted identity for a happening. Type, participants, occurrence, and other properties are role and attribute Assertions about it. Keep the universal parent roles with typed subroles, Event-to-Event relations, and the disclosure-safe projection rules (omissible role, incomplete shell, suppression).
12. Resolution hypothesis: one reversible `same_as` hypothesis mechanism shared by identity stubs and Events. It has immutable member sets, evidence, and proposing authority. Lifecycle: candidate, accepted (which mints a separate composite ID), rejected, withdrawn, superseded. Accepted sets are disjoint within one ResolutionEnvironment. Hypotheses are never transitively closed. An identity acceptance additionally records a clearance level, `recall` or `disclosure`; only disclosure-cleared composites may reach response-affecting context. Withdrawal invalidates whatever was derived under the environment that contained it. The ResolutionEnvironment stamp stays. Drop per-scope propagation rules beyond what the clearance split needs. Autonomous identity is the first capability planned after the successor lands, so the clearance split must be present and clearly specified.
13. Task: an agent-authored action intent with one or more trigger conditions. Firing is recorded per condition. Lifecycle: proposed, active, completed, cancelled, superseded, erased. A trigger condition fires only while its Task is active. Descriptive Event occurrence never fires. This merges the original Task and Trigger records.

## Lifecycle mechanics (statements.md)

- Identity-bearing records are immutable. Every change is an appended transition.
- Every transition names its target and the one transition (or the creation record) it follows. A write whose named predecessor is not the target's current head is rejected. This prevents lost updates between a turn and a background job, and it replaces most of compare-at-commit.
- Two accepted transitions naming the same predecessor are a fork. A single writer cannot produce one; only an import or a future multi-writer merge can. The projection marks the target conflicted, and a later transition resolves it. Do not specify more than that.
- Erasure is terminal. Replay never reconstructs erased payload.
- Drop recorded denied-decision records, per-object prior-state matrices beyond one short table, and the damaged-import quarantine procedure.

## Privacy and provenance (privacy-and-provenance.md, belief.md)

- Transmission principles: `public`, `attributed`, `in_confidence` (teller-only unless demonstrated witness), `include(S)`, `exclude(S)`. Composition by conjunction, universal quantification over the audience, and fail-closed on unknown identity all stay. Consent, purpose limitation, and reciprocity are deferred.
- Witness evidence: keep the assurance-kind table and the scoped-widening rule. This is the fix for issue #123 (the present set conflating audience with participation).
- Subject guard: keep the algorithm and the guard scenarios. Tighten the prose. Remove the per-call recording of policy versions that the context manifest and log already imply.
- Zero residue and audience-safe support: keep.
- Influence: replace InfluenceEnvelopes with context-manifest projections, as defined above.
- Erasure keeps: the authority table (teller, operator); the envelope/payload split; tombstones; dependant invalidation over manifests and input edges; the offline operation under the writer lock; and crash semantics in a few sentences. The payload commitment hash is salted, and the salt is deleted with the payload, so an erased low-entropy payload cannot be confirmed by guessing.
- Erasure ledger: a small append-only, hash-chained file holding only IDs and positions (never content or unkeyed digests). It is kept apart from the log and copied to a second location on every erasure. Every boot reconciles the store against the ledger before serving: tombstoned payloads still present are deleted, and snapshots older than the ledger head are scrubbed or rejected. Restoring an old backup therefore needs no special procedure. A missing or unverifiable ledger refuses boot unless the operator passes an explicit flag acknowledging possible resurrection. This replaces the restore state machine, the storage-class taxonomy, and all operational fixtures.
- External copies: eval packages, exports, console downloads, and debug captures built from a real log are recorded as copy records, so an erasure reports the copies it cannot reach. One short paragraph.
- Delete the inter-agent status and revocation section. List inter-agent exchange as deferred.
- Belief: keep the genesis evidence list minus reliability observations (deferred), expression versus corroboration, dependence, the support projection's two modes, promotion and withdrawal, contest and mechanical contradiction, and the agent-facing ordinal.

## Time (time.md)

Keep the typed temporal values, the validity/observation/recording split, modality, recurrence intent with its exceptional-date policies, correction versus world change, and staleness and volatility (shortened). Replace the qualitative interval algebra with Event-bounded validity: an Assertion's validity bound may be `before`, `after`, or `during` a named Event, and it resolves when that Event's occurrence becomes known. Richer qualitative reasoning is deferred. Task and triggers follow the object model above.

## Relations (relations.md)

Keep versioned definitions, deprecate-and-alias, alias-on-read, and "context does not become hidden schema". Replace operator-governed activation with agent coinage under critics, which matches the current CONTRIBUTING seed-ontology principle that social semantics are the agent's to coin at runtime:

1. Coining requires a description, domain, range, and an example. The description is embedded.
2. A new relation whose name or description lands near an existing definition with compatible endpoints receives a teachable error naming the existing relation. The agent either uses it or states how its relation differs.
3. The relations already in use for the relevant entity kinds are rendered at the point of writing, so the agent reuses before it coins.
4. A new relation is `provisional` until used across a policy-defined number of Occasions. A background job may alias a provisional relation into an established one without the operator. Aliasing an established relation needs the operator.
5. A functional or exclusive cardinality affects mechanical contradiction detection only after the operator confirms it.

Entity kinds, roles, Event types, frames, modalities, and transmission principles stay operator-governed.

## Write path (verified-write.md, write-surface.md)

- Proposal states: `proposed`, then `published`, `source_only`, or `abandoned`. Attempts and retries are recorded as attempts. Publication is atomic, and the predecessor check applies at commit. A crash mid-proposal reruns extraction; both attempts are recorded, so replay stays deterministic.
- Keep the hard critics (proposition, assertion, attestation, grounding, Event, Task, derivation, audience, episodic wall), soft critics, teachable errors, excluded operations, and the cost boundary.
- Stage 1 decides between two candidate write surfaces. The spec presents both concisely as candidates and does not pick:
  - A: `record` stores the Occasion, an extractor proposes structure, and the agent reviews it in a later block (`amend`, `drop`, `accept`);
  - B: the agent writes typed claims directly, and the same hard critics return teachable errors.
- Keep `claim` for non-Occasion sources and the required caller decisions (frame, default transmission, relayed teller, source kind).

## Query surface (query-surface.md)

Reads are pure functions of (frontier, audience, ResolutionEnvironment). Audience resolution happens before candidate ranking. Access accounting is the set of context-manifest entries: content actually rendered into a context, never hidden candidates. Remove the read state machine and the authorisation-input digest. Keep structural queries, labelled search lanes, rank fusion by position, explicit `inspect`, operator traces, and non-disclosing errors.

## Off-turn work (off-turn.md)

- Keep the authority classes, idempotent job keys, change-driven marks instead of whole-store sweeps, dormant contested items, and scheduling that drains due Task triggers before maintenance.
- Workers are stateless clients of the server API: threads, other processes, or other machines. The server remains the single writer and hands out work in memory. Assignments are never logged. A worker submits its result as a proposal carrying the job key and the heads it read. The first valid result for a key commits. A late duplicate, or a result computed from changed inputs, is recorded as stale together with its model call, so audit and influence stay complete. A dead worker's assignment times out and the job is reassigned. Remove leases, the lease state machine, and poison handling for competing workers. Keep a bounded retry and a terminal failure state that is visible to the operator.
- Exploration and proactive initiation stay deferred, each with a short paragraph.

## Memory typology and two traces

Trim both. The episodic wall becomes a manifest check: a semantic write is rejected when any context manifest in its Activity ancestry contains an `episodic_reconstruction` Artefact. Original evidence in the same context does not cancel it. Keep the four boundary scenarios (direct, mixed, note-mediated, source-only control). Working-note taint is a manifest projection. Keep the self and directives as configuration.

## Scenarios replace fixture grammar

Delete the schema-neutral fixture record grammar, the vector template language, the ID-prefix table, the canonical JSON and CBOR vectors, and the operational fixtures. Each chapter instead ends with a table of scenarios:

| ID | Scenario | Expected result |
|---|---|---|

IDs are stable kebab-case (`compound-utterance`, `identity-severance`, `hidden-support`). Keep every original scenario whose mechanism survives this brief, compressed to one line of setup and one line of result. Drop scenarios that test only removed machinery (restore state machine, lease races, read-digest retries, damaged-import forks). Add scenarios for new decisions where obvious (`restore-old-backup-reconciles-at-boot`, `missing-ledger-refuses-boot`, `near-duplicate-relation-teaches`, `provisional-relation-auto-alias`, `page-span-citation`, `stale-worker-result`, `lost-update-rejected`). The README states the convention: a future Rust test carries the scenario ID, and a CI check fails when a documented ID has no test or a test ID is undocumented. The check itself is future work.

## Registers (evolution.md, confidence.md, coverage.md, lineage.md)

- evolution.md becomes four milestones plus a deferred-capability list:
  1. Evidence (the former stage 1): extraction on the real corpus, a head-to-head comparison of write surfaces A and B, query classification over real turns, a witness-assurance audit for #123, and budgets for log growth and the console replica (#66). Committed fixtures derived from the operator's real data must be anonymised to invented placeholders.
  2. Reference model: an in-memory Rust model of the object model, with the scenario tables as its tests.
  3. Vertical slice: the audience-resolved read and write path, erasure with the ledger and boot reconciliation, and stateless off-turn workers.
  4. Policies, then genesis: initial temporal, Event and relation, identity, and support policies; a genesis freeze review; a rehearsal; the first real genesis.
  Each milestone states what it produces, what would falsify the design, and what it blocks, in a few sentences. The deferred-capability list names each capability with its reopen condition and marks autonomous identity as the first planned after the successor lands. Remove the dependency register, owner register, capability census, selection-record schema, executable, oracle, invariant, and handoff registers, and the four-status vocabulary. A capability is either in the genesis design or deferred.
- confidence.md: keep the evidence map (claim, grade, source), a short list of open questions, and "Claims deliberately not made". Remove the obligation registers and the other registers.
- coverage.md: keep the eleven-failure table with columns failure, classification, mechanism, evidence required, and residual risk, reclassifying where this brief changes a mechanism. Keep the Regressions and "Mitigations the design erases" sections. Reduce the issue mapping to one line per issue (issue, effect, where in the design). Remove the ownership vocabulary and machine-integrity rules.
- lineage.md: update links only.

## Prose and repository rules

- Follow `CONTRIBUTING.md` in the repository root, especially the Documentation section. Prose is Australian English; code identifiers keep their US spelling. Use the plain technical register: short declarative sentences, present tense, third person, and active voice. No "you" or "we". Headings in sentence case. Oxford comma. No bold or italics used for emphasis. No em-dash asides; use a colon or split the sentence. Do not hard-wrap lines.
- Present tense describes the proposed design as normative. Do not narrate the rewrite: no "previously", "this used to", or "now simplified". The original is in git.
- Keep citations into `research/` and `../docs/` wherever a claim relies on evidence. Keep the honest grading language ("design synthesis", "established", "unvalidated"). `docs-future/research/` is exempt and must not be modified.
- This is a simplification, not a trim for length. A claim, caveat, or evidence citation that this brief does not remove survives in compressed form. When unsure whether to cut something, keep a one-sentence version and list it in your report.

## Working rules for agents

- Edit only the files assigned to you. Other agents are editing other files in the same working tree at the same time.
- Never run git commands that change state: no `stash`, `checkout`, `reset`, `add`, `commit`, or `restore`. Read-only git is fine, for example `git show HEAD:docs-future/statements.md` to see the original.
- Link to other chapters by file and the heading you expect. A final pass reconciles anchors.
- Your final message is a report containing: the files and their approximate new size; the headings you defined; what you cut; what you kept that you were unsure about; any place where the brief was ambiguous or seemed wrong, and what you did about it.

## Size guidance

These are rough targets to aim for, not hard limits. statements.md about 20KB; privacy-and-provenance.md about 16KB; each other chapter 5–10KB; evolution.md, confidence.md, and coverage.md about 10–15KB each. The whole tree excluding `research/` should land somewhere between 120KB and 180KB.
