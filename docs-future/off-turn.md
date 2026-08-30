# Off-turn work

Off-turn work executes durable jobs when no conversational turn is active. Jobs perform mechanical projections, propose derived Assertions, or propose retractions. These authorities are separate. A job receives only the permissions declared by its job kind.

[The assertion model](statements.md) owns Activity, Assertion, Attestation, and Derivation identity. [Verified writes](verified-write.md) owns proposal publication. [Privacy and provenance](privacy-and-provenance.md) owns transmission and influence rules. Off-turn jobs refer to those records and do not define substitute identities.

## Authority classes

A `mechanical_projection` job updates a rebuildable projection from recorded events. It cannot append an Assertion, Attestation, Perception, or retraction transition.

A `derived_assertion` job can open a verified-write proposal with an Activity that produces a Derivation as its source. It records typed inputs, criterion and implementation versions, ontology and policy versions, unified ResolutionEnvironment, assumptions, and source head sequence. It cannot claim human testimony or publish without the ordinary critics.

A `retraction_proposal` job can propose withdrawal or invalidation for a record it did not author. It records the target, authority basis, evidence, and requested transition. It cannot substitute a replacement value. The authorised teller, subject, operator, or policy reviewer accepts the transition through the applicable owner surface.

All classes can append job transitions and Activity records. None can widen an audience, cross the episodic wall, modify the self slot, activate schema, accept an identity merge, or bypass a critic.

## Job identity and key

Current maintenance passes repeatedly inspect stored prose and supply the observed workaround and scaling constraint ([maintenance passes](../docs/maintenance-passes.md), [current write machinery](../docs/write-path.md)). Durable-activity and large-system research support recording nondeterminism and avoiding constant per-fact sweeps ([welding research](research/2026-07-24/lanes/welding.md), [scaling survey](research/2026-07-24/lanes/survey-giants.md)). The stable key, leasing, compare-at-commit, supersession, and poison protocol below are design synthesis with no direct local implementation analogue. Stage 11 crash, race, and idempotency tests are its evidence gate.

Each logical job has a stable job ID and a unique job key:

```text
(job_kind, target_id, target_head_sequence, policy_version, implementation_version)
```

The target head sequence states the source state the job intends to process. Enqueueing the same key is idempotent. A changed evidence, time, schema, policy, or identity head creates a new key and supersedes obsolete pending work when the job kind declares that relationship.

The job record carries its authority class, transmission domain, priority, attempt bound, poison policy, and compare-at-commit inputs. A lease record carries the worker ID, attempt number, acquisition sequence, and expiry. Lease expiry permits another worker to append a new attempt. It never removes the abandoned attempt.

## Job state machine

Every transition appends an event. The fold derives current state by job ID.

| State | Appended record | Permitted successor |
|---|---|---|
| `queued` | job key, target head, authority, versions, priority, and retry policy | `leased`, `cancelled`, `superseded` |
| `leased` | worker, attempt, and lease expiry | `running`, `queued`, `cancelled`, `superseded` |
| `running` | Activity start and consumed input IDs | `prepared`, `retry_wait`, `poisoned`, `cancelled`, `superseded` |
| `prepared` | deterministic projection delta or verified-write proposal ID, plus expected commit heads | `committing`, `retry_wait`, `cancelled`, `superseded` |
| `committing` | compare-at-commit attempt | `completed`, `stale`, `retry_wait`, `poisoned` |
| `retry_wait` | failure class, attempt count, and next eligible time | `queued`, `poisoned`, `cancelled`, `superseded` |
| `stale` | observed head mismatch and replacement key, if needed | `superseded`, `queued` |
| `completed` | output IDs, committed head, and completion digest | terminal |
| `cancelled` | actor and reason | terminal |
| `superseded` | replacement job ID and reason | terminal |
| `poisoned` | terminal failure class, diagnostics, and operator disposition | terminal |

A worker must hold the latest unexpired lease before it appends `running` or `prepared`. Completion compares the target, policy, schema, identity, and relevant output heads with the values recorded in `prepared`. A mismatch appends `stale`; it never commits a result against changed premises. Mechanical projection commits are idempotent by job key and output version. Derived and retraction outputs use the verified-write proposal's stable IDs and atomic publication protocol. These race semantics are normative synthesis, not behaviour inherited from the current passes; Stage 11 must exercise lease expiry immediately before prepare and commit, duplicate workers, cancellation races, and a crash on both sides of the commit marker.

A crash before `prepared` leaves an expired lease that another worker can retry. A crash after `prepared` reuses the recorded Activity result. A crash during commit resolves by reading the output commit marker before another attempt. A worker never repeats a recorded nondeterministic call solely because it lost its lease.

Transient infrastructure failures enter `retry_wait` with bounded exponential delay. Deterministic critic failures do not retry until an input or version changes. Exhausted attempts enter `poisoned`. Poisoned jobs remain visible to the operator, do not block unrelated keys, and require an explicit discard, replacement, or policy change.

Cancellation prevents a future commit. If output committed first, cancellation cannot erase it and must use the canonical retraction or correction transition. Supersession links the old and replacement jobs. The old job cannot commit after the supersession sequence because compare-at-commit includes the job state head.

## Queue marks

Writes enqueue work from specific changes rather than from whole-store sweeps.

| Mark | Relevant change | Job response |
|---|---|---|
| `recompute_owed` | a derivation input or criterion version changes | derive again against the new head |
| `support_weakened` | visible support, dependence, or reliability changes | re-evaluate the affected derived result |
| `resolution_invalidated` | an identity or Event resolution is withdrawn | rebuild affected projections and derivations |
| `validity_boundary_due` | recorded time reaches a declared boundary | apply the versioned temporal policy |
| `source_only` | a proposal ended without structure | perform the one bounded structuring retry allowed by policy |
| `pending_ingest` | a source-first ingest segment is durable | process the named segment under its source audience |
| `working_review_due` | a working item reaches its review condition | propose promotion or discard |
| `episode_due` | a session meets the optional episode policy | compose under the episodic wall |

A tick over empty queues costs a queue read. Whole-store canaries and replay audits remain diagnostic operations. They do not become routine curation passes.

## Dormant contested items

A contested Assertion, Event co-reference, or identity hypothesis runs classification once for a named evidence and policy head. If classification cannot resolve it mechanically, the item enters `dormant_contested`. Time alone does not requeue it unless the relevant policy declares a temporal condition.

The store requeues a dormant item only when one of these dependencies changes:

- a cited Assertion, Attestation, Perception, or source arrives, changes status, or is retracted;
- the applicable contradiction, support, or promotion policy changes version;
- a relevant relation, role, kind, modality, or Event definition changes version;
- either component of the unified ResolutionEnvironment changes;
- a declared temporal boundary is reached.

The wake record names the changed dependency and creates a new job key. Repeated heartbeat ticks do not reconsider an unchanged contest.

## Scheduling

The scheduler drains due Triggers before maintenance jobs. A Trigger is a recorded commitment rather than maintenance work. Exhausting the tick budget drops or defers maintenance capacity and does not defer a due Trigger.

Mechanical jobs consume no model budget. Judgement jobs use a bounded per-tick model budget. Every model call is a durable Activity whose recorded result replay consumes. Priority cannot override audience, authority, lease, retry, or compare-at-commit checks.

## Exploration

Exploration is an open experiment and is disabled by default. It is not part of the remedy for failed extraction, contested items, or missed maintenance. Those cases have explicit marks and jobs.

When enabled, exploration operates only within one compatible transmission domain selected before candidate pairing. It cannot combine records merely by intersecting incompatible audiences after inference. Its output is a working item with complete influence lineage. It cannot publish an Assertion, promote itself, initiate a message, or consume capacity reserved for due Triggers and marked jobs.

The experiment has a fixed budget, a yield gate, a privacy non-interference gate, and a disable switch. Its cost does not fall with store growth, so it remains optional even if the safety gates pass.

## Agent-initiated work

An off-turn message requires an explicit initiating event such as a due Trigger, a commitment, or an operator exception request. Spare scheduler capacity is not an initiating event. The audience evaluator checks the proposed complete recipient set and fails closed.

The outbound message creates an Occasion with the recipients supported by connector witness evidence. Assertions made by the agent use agent-authored Attestations only where the assertion model permits them. The operation cannot cross an unresolved identity boundary because no live challenge-response is available.

The policy that decides which eligible item warrants interruption remains an open experiment. The job and audience invariants apply before that policy can be enabled.

## Replay

The fold remains model-free. Replay folds job transitions, leases, Activities, proposal states, and commit markers. It never reruns a model, infers a missing historical field, or changes the meaning of an old job key after a policy version changes. New job kinds and policies use additive versions.
