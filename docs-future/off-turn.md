# Off-turn work

Off-turn work runs jobs when no conversational turn needs the capacity. Jobs update mechanical projections, structure recorded Occasions, propose derived Assertions, propose retractions, or alias provisional relations. These authorities are separate. A job receives only the permissions its job kind declares.

[The object model](statements.md) owns object identity and the predecessor rule, [verified writes](verified-write.md) owns publication, and [privacy and provenance](privacy-and-provenance.md) owns transmission and influence.

Current maintenance passes repeatedly inspect stored prose, which is both the observed workaround and the scaling constraint ([maintenance passes](../docs/maintenance-passes.md), [current write machinery](../docs/write-path.md)). Durable-activity and large-system research support recording nondeterminism and avoiding constant per-fact sweeps ([welding research](research/2026-07-24/lanes/welding.md), [scaling survey](research/2026-07-24/lanes/survey-giants.md)). The job and worker model below is design synthesis with no direct local analogue.

## Authority classes

A `mechanical_projection` job updates a rebuildable projection from recorded events. It cannot append an Assertion, Attestation, Perception, or retraction transition, and it makes no model call.

A `structuring` job opens a verified-write proposal over one or more existing Occasions. It serves deferred structuring, when that schedule is chosen ([write surface](write-surface.md#what-the-comparison-measures)), and the bounded retry after `source_only`. Its proposals face the same [hard critics](verified-write.md#hard-critics) as a turn's, including teller binding and the principle floor.

A `derived_assertion` job opens a verified-write proposal whose output carries a `derivation` Attestation with its full inputs ([derivation inputs](verified-write.md#derivation-inputs)). It cannot claim testimony or skip the critics.

A `retraction_proposal` job proposes withdrawal or invalidation of a record it did not author, naming the target, authority basis, evidence, and requested transition. It never substitutes a replacement value. The current system reached this rule after finding that a date the agent wrote at append time was examined by nothing: across 405 runs, 334 appends carried an authored date against 112 that extraction resolved, and only the latter were checked ([`research/2026-08-06/current-system-fixes.md`](research/2026-08-06/current-system-fixes.md)). A withdrawn value disarms; a substituted one arms something else under the original author's authority. The authorised teller, subject, operator, or policy reviewer accepts the transition.

A `relation_alias` job aliases a provisional relation into an established one under the rules in [relations](relations.md#coining-under-critics). It cannot alias an established relation, which needs the operator.

No class can widen an audience, cross the episodic wall, modify the self or the directives, change an operator-governed definition, accept an identity hypothesis, or bypass a critic.

## Job keys

Each logical job has a key:

```text
(job_kind, target_id, target_head, policy_version, implementation_version)
```

`target_head` is the target's transition head. Enqueueing an existing key is idempotent. A change to the target's evidence, time, schema, policy, or identity moves its head and produces a new key; work under the old key needs no cancellation, because its result fails the predecessor check. The pending set is a projection of committed changes, so a restart rebuilds it from the log.

## Workers

Workers are stateless clients of the server API: threads, other processes, or other machines. The server stays the single writer.

1. The server hands out a pending job in memory, with its key and the inputs it needs. Assignments are never logged.
2. The worker reads through the server API and requests any model call through it. The server renders the context through the [single rendering choke point](statements.md#context-manifest), records the call and its manifest, and returns the result. A worker never renders content into a model context or calls a model directly, so manifest completeness stays a property of one component and every model call is recorded when it is made.
3. The worker submits its result as a proposal carrying the job key, the heads it read, and the IDs of the model calls it used. The server applies the ordinary critics and predecessor check. The first valid result for a key commits.
4. A late duplicate, or a result whose read heads are no longer current, is recorded as `stale`. It publishes nothing. Its model calls are already recorded, so audit and influence stay complete.
5. An assignment with no submission within its timeout is handed out again. Model calls the abandoned attempt made remain recorded as Activities with no committed output.

A submitted failure is recorded with its class and attempt number. Transient failures retry with bounded exponential delay. A deterministic critic rejection waits for an input or version change, which gives a new key. When recorded failures plus the timeouts seen since the server started exhaust the bound, the server records a terminal `failed` outcome with diagnostics. A failed job stays visible to the operator, blocks no unrelated key, and needs an explicit discard, replacement, or policy change. A restart resets only the in-memory timeout count.

Mechanical projections are idempotent by key and output version; derived and retraction outputs publish atomically. A committed result is undone only by a retraction or correction transition. With no worker configured, pending work waits, and a later worker takes it up without reinterpreting recorded state.

## Queue marks

Writes enqueue work from specific changes rather than from whole-store sweeps.

| Mark | Relevant change | Job response |
|---|---|---|
| `recompute_owed` | a derivation input or criterion version changes | derive again against the new head |
| `support_weakened` | visible support or dependence changes | re-evaluate the affected derived result |
| `resolution_invalidated` | an identity hypothesis is withdrawn | rebuild affected projections and derivations |
| `contradiction_detected` | publication, or an arriving Assertion, matches a registered contradiction rule ([contest](belief.md#contest-and-contradiction)) | classify the mechanical contest |
| `source_only` | a proposal ended without structure | a `structuring` job performs the one bounded retry the policy allows |
| `relation_provisional` | a provisional relation reaches its review condition | propose an alias or leave it provisional |

A tick over empty queues costs a queue read. Whole-store canaries and replay audits stay diagnostic and never become routine curation. Whether the queues stay short under real traffic is unmeasured. The claim that structural deduplication and cross-audience merging leave consolidation entirely rests on extraction convergence, which the evidence milestone measures.

## Dormant contested items

Dormant handling applies only to mechanical contests. A contest is classified once per evidence and policy head. If that cannot resolve it mechanically, it becomes `dormant_contested` and is requeued only when a dependency changes:

- a cited Assertion, Attestation, Perception, or source arrives, changes status, or is retracted;
- the applicable contradiction, support, or promotion policy changes version;
- a relevant relation, role, kind, modality, or Event definition changes version;
- an identity hypothesis in the recomputed ResolutionEnvironment changes;
- a declared temporal boundary is reached.

Time alone never wakes it unless the policy declares a temporal condition. The wake names the changed dependency under a new job key.

## Scheduling

The scheduler fires due Task trigger conditions before it hands out maintenance. The genesis Task action, `wake_turn`, its creation-time audience check, and what a firing renders are defined in [time](time.md#occurrence-and-task). A due trigger is an initiating event at genesis, so reminders work.

A message that does not answer an inbound Occasion, including a `wake_turn` reply, cannot use challenge-response in the moment. Its audience must resolve from clearances and witness evidence already recorded, and an unresolved identity boundary fails closed. A trigger is a commitment, so an exhausted tick budget defers maintenance and never a due trigger. This order prevents a commitment starved by tidying. Mechanical jobs use no model budget, and judgement jobs draw from a bounded per-tick budget. Priority never overrides audience, authority, retry, or predecessor checks.

## Deferred: exploration

Exploration would sample structure for connections no mark names, a real gap in a mark-driven store. It is not the remedy for failed extraction, contests, or missed maintenance, which have marks. If enabled, it pairs candidates only within one transmission domain chosen before pairing, yields working notes with complete manifests, uses only leftover budget, and cannot publish, promote itself, or send a message. Its cost does not fall as the store grows, and its yield at this instance's size is unmeasured. It reopens when a fixed-budget trial shows useful yield without privacy interference.

## Deferred: proactive initiation

Proactive initiation is initiation without a Task: the agent deciding on its own judgement to message someone. A Task-triggered `wake_turn` is not proactive initiation and is in the genesis design. Spare capacity is never an initiating event. If enabled, its audience resolves from recorded clearances as in [scheduling](#scheduling) and fails closed. The salience judgement is unsettled, and the capability reopens when a salience and interruption policy is proposed and evaluated.

## Deferred marks

Four marks belong to deferred capabilities and exist only once their capability is enabled:

- `pending_ingest` fires when a source-first ingest segment is durable, and a job processes the segment under its source audience ([bulk ingestion](memory-typology.md#conversational-artefacts-are-not-bulk-ingestion)).
- `episode_due` fires when a session meets the generated-episode policy, and a job composes an episode under the [episodic wall](two-traces.md#the-episodic-wall).
- `validity_boundary_due` fires when recorded time reaches a declared validity boundary, under volatility automation ([staleness and volatility](time.md#staleness-and-volatility)).
- `working_review_due` fires when a working note reaches a review condition, under working-note promotion review. Explicit promotion by the agent is in the genesis design.

## Replay

The fold stays model-free. Replay folds recorded results, stale results, failures, Activities, and publication records. It never reruns a model, infers a missing historical field, or changes the meaning of an old job key after a policy version changes. New job kinds and policies arrive as additive versions.

## Scenarios

| ID | Scenario | Expected result |
|---|---|---|
| `stale-worker-result` | A worker reads an Assertion at head H, and a turn transitions it before the worker submits. | The result is recorded as `stale` with its model call. Nothing publishes, and the new head yields a new key. Unlike `publication-stale-predecessor`, the result is not reworked: the job key is superseded and a fresh job runs. |
| `duplicate-worker-result` | Two workers submit valid results for the same key. | The first commits. The second is recorded as `stale` with its model call. |
| `worker-timeout-reassign` | A worker dies mid-job without submitting. | The assignment times out and the job is handed out again. The assignment itself leaves no record; any model call it made stays recorded with no committed output. |
| `retry-exhausted-visible` | A job fails transiently until its retry bound is exhausted. | A terminal `failed` outcome is recorded and shown to the operator. Unrelated keys keep running. |
| `critic-rejection-no-retry` | A derived result fails a hard critic deterministically. | The job does not retry until an input or version changes. |
| `idempotent-enqueue` | The same change is marked twice. | One pending job exists for the key. |
| `restart-rebuilds-pending` | The server restarts with pending and assigned jobs. | The pending set is rebuilt from committed records, and assigned jobs are pending again. |
| `no-worker-waits` | No worker is configured when a mark is enqueued. | The job stays pending and nothing runs. A worker added later picks it up. |
| `mechanical-cannot-assert` | A `mechanical_projection` job attempts to append an Assertion. | The append is refused. |
| `retraction-no-substitution` | A `retraction_proposal` job proposes a replacement date for an authored date. | The replacement is refused. Only withdrawal can be proposed. |
| `alias-established-refused` | A `relation_alias` job proposes aliasing an established relation. | The proposal is refused. |
| `dormant-contest-no-tick` | A dormant mechanical contest sees many ticks with no dependency change. | No job is enqueued and no model call is recorded. |
| `dormant-contest-wakes` | A new Attestation arrives for a dormant contest. | The contest is requeued under a new key naming the changed dependency. |
| `due-trigger-scheduled` | An active reminder Task's trigger comes due in a tick with a full maintenance queue and budget for one job. | The firing is recorded and the turn starts. The maintenance job is deferred to the next tick. The turn behaves as `trigger-wakes-turn` in [time](time.md#occurrence-and-task). |
| `wake-turn-no-challenge` | A `wake_turn` conversation includes a participant whose identity cannot be resolved from recorded clearances. | The audience check fails closed. No challenge-response is attempted. |
| `no-task-no-initiation` | Spare capacity exists and no Task is due. | No turn starts and no outbound Occasion is recorded. |
| `structuring-retry-binds-teller` | A `structuring` job retries a `source_only` Occasion, proposing Quinn as teller of Rowan's span. | The attestation critic rejects that item. Items whose tellers are the span authors publish. |
| `contradiction-mark-classified` | A publication matches a registered functional-relation rule against a live Assertion. | A `contradiction_detected` mark is appended, and one classification job runs for its key. |
| `empty-tick-cheap` | A tick runs over empty queues. | It costs one queue read and no model call. |
