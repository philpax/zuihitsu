# Identity

Identity resolution links person entities, including permanent platform stubs, without rewriting them. This chapter owns the identity resolution hypothesis: the single reversible `same_as` mechanism. It also owns recall and disclosure clearance, the designated primary member, the derived ResolutionEnvironment, and the one-handle surface. The [object model](statements.md) owns the identity and lifecycle of [Entities](statements.md#entity), Occasions, Assertions, and Attestations. Resolution hypotheses are identity-only. Events keep stable identity and have no co-reference mechanism at genesis ([events and roles](events-and-roles.md#co-reference)).

Hard equivalence is too strong. Linked-data deployments show that transitive closure amplifies one incorrect link across an entire class, including a measured case that collapsed names for more than 177,000 distinct entities. Record-linkage research shows that shared attributes are unsafe evidence, because copied facts violate the independence assumption. Relational evidence is harder, but not impossible, to forge ([research lane](research/2026-07-24/lanes/identity-belief.md); [confidence](confidence.md#evidence-map)).

## Permanent stubs

A connector mints a platform stub from the most stable identifier the platform supplies. A stub is an [Entity](statements.md#entity) of a person kind with connector scope: it has a permanent ULID, a connector and platform scope, creation evidence, and the Entity lifecycle. A merge never consumes, renames, or repurposes it. When a connector's identifier rotates, the connector appends an alias or replacement record and retains the original stub.

A stub is not proof of a person. It is the stable source identity against which Occasions, witness evidence, and authorisation decisions were recorded. Reads may present a resolved person view, but stored provenance, such as a testimony Attestation's teller or an Occasion's participants, continues to name the original stub.

The agent also mints person entities, for people without an account and for a person it knows before any connector has seen them. An agent-minted person entity has no connector scope. It never becomes a stub, and a stub never becomes an agent-minted entity. The two come together only through a resolution hypothesis whose member set holds both, for example the agent's `person/rowan` and the connector's stub for Rowan's chat account. `same_as` across entity kinds is refused.

## Resolution hypotheses

A resolution hypothesis proposes that a set of person entities denote one person. Its immutable creation record holds:

- a minted hypothesis ID;
- an ordered, duplicate-free member set of at least two person entities or prior composites;
- evidence locators with their evidence kinds and dependence lineage;
- the proposing authority and the Activity or Occasion that proposed it;
- the frontier it was proposed at.

The creation record carries no status or clearance. Its lifecycle folds from appended transitions under the [lifecycle mechanics](statements.md#lifecycle-mechanics).

| Transition | Required fields | Fold result |
|---|---|---|
| none | | `candidate` |
| `accepted` | authority, evidence, policy version, a separately minted composite ID, a clearance level, and a designated primary member | `accepted`; the composite is live in every environment computed while the acceptance stands |
| `clearance_changed` | authority, evidence, new clearance level | `accepted` at the new level; the composite ID is unchanged |
| `primary_changed` | operator authority, new primary member | `accepted` with the new primary; the composite ID is unchanged |
| `rejected` | authority, reason, evidence | `rejected`; members stay separate |
| `withdrawn` | authority, severance evidence | `withdrawn`; the composite stops being live |
| `superseded` | authority, reason, replacement hypothesis ID | `superseded`; the replacement folds independently |

`rejected`, `withdrawn`, and `superseded` are terminal. A transition cannot change the member set or reuse a composite ID. A different member set is a new hypothesis. Binary proposals are the initial policy. The member set is n-ary from genesis so that later evidence can resolve a set without chaining pairwise equivalences.

Acceptance authorises one composite view. It does not assert universal equality. The composite ID is a new stable identity, and every member keeps its ID, Attestations, and history. The literature supports graded, revisable links and non-destructive merge overlays. The record, transitions, and clearance split are design choices ([research lane](research/2026-07-24/lanes/identity-belief.md)).

## Overlap and non-transitivity

Hypotheses are never transitively closed. Candidates `{a, b}` and `{b, c}` may coexist with separate evidence. The acceptance rules are:

1. Accepted member sets are disjoint at every frontier. An entity belongs to at most one live accepted composite.
2. An acceptance that overlaps a live accepted set is rejected unless the same atomic batch withdraws or supersedes the conflicting hypothesis. The system never infers `{a, b, c}` from `{a, b}` and `{b, c}`.
3. Overlapping candidates stay inspectable and may collect evidence. They do not affect ordinary resolved reads.

## Recall and disclosure clearance

An identity acceptance records one of two clearance levels. They are ordered capabilities, not points on a confidence score.

- `recall` permits cross-member content to enter an internal Activity whose output cannot reach response-affecting context. At genesis the only such Activity is an operator diagnostic, which never feeds a turn.
- `disclosure` additionally permits cross-member content to enter response-affecting context for a resolved audience, subject to [privacy evaluation](privacy-and-provenance.md).

Response-affecting context is any context whose output can reach a participant or an action: a conversational prompt, retrieval ranking for a turn, a generated summary, a tool argument, a Task, or an initiated action. Candidate hypotheses have no clearance, and at genesis a candidate is rendered only into an operator diagnostic.

Tentative unified recall is not harmless: content from another member that enters a model call can shape an irreversible disclosure even when the final text omits the source. The genesis rule is therefore structural. The context assembler renders cross-member content of a composite below `disclosure` only into operator diagnostics, recorded in the [context manifest](statements.md#context-manifest) like any other render. No recall-cleared content reaches an Activity that feeds a turn, so the recall-restriction [influence projection](privacy-and-provenance.md#influence-from-context-manifests) is deferred until autonomous identity renders recall-cleared content off-turn ([evolution](evolution.md#deferred-capabilities)). Enabling it changes no stored record.

Disclosure clearance is evaluated per audience member. A composite cleared at `disclosure` lets a member's resolved identity satisfy a transmission principle that names any member of the composite. Every other audience member is evaluated separately, so clearance for one account never clears a group that contains other people. The universal quantification and fail-closed treatment of unknown identity belong to [privacy and provenance](privacy-and-provenance.md#transmission-principles).

A passive similarity score never grants `disclosure`. Only operator confirmation or a connector-authenticated challenge-response under a registered policy grants it. A downgrade from `disclosure` to `recall` makes every derivation that relied on disclosure-level clearance non-current, as a withdrawal would. A disclosure already made cannot be undone, which is why response-affecting use requires `disclosure` first.

## Writes through a composite

Each accepted composite designates one primary member. The operator names it at acceptance and may change it with `primary_changed`. For a composite that joins an agent-minted person entity with one or more stubs, the agent-minted entity is the usual primary, because its handle is the readable profile.

A write through the composite's handle lands on the primary member. The Assertion's subject or object is the primary's ULID, not the composite ID. The Attestation records its frontier, and the environment recomputed at that frontier names the composite's head transition and therefore its primary at the time. Testimony tellers and Occasion participants still name the stub that produced the span ([teller binding](statements.md#attestation)).

Every write made through a composite is therefore enumerable from the log: it is an Attestation on an Assertion about the primary whose recorded frontier falls while the composite was accepted. On withdrawal or supersession, the Assertions stay on the primary, and nothing moves automatically. The transition appends a re-homing review item that lists those writes for the operator. The operator may re-home any of them by superseding the Assertion onto another member, or leave it on the primary. A `primary_changed` transition moves nothing either: earlier writes stay on the earlier primary.

## Resolution environments

A ResolutionEnvironment is a derived read concept, not a stored record. At a frontier, it is the set of live accepted hypotheses, each with its composite ID, head transition, clearance level, and primary member, together with the overlap and clearance policy versions in force. Reads, and every [derivation Attestation](statements.md#attestation) that depends on resolved identity, record their frontier, and the environment is recomputed from the log at that frontier. An operator diagnostic may additionally admit named candidates as a parameter recorded by its Activity. Reads are pure functions of the frontier, the audience, and this environment, as the [query surface](query-surface.md) specifies.

Environments at frontiers after a withdrawal or supersession no longer contain the composite. A derivation Attestation whose typed inputs span more than one member of the composite, and whose recorded frontier falls while the composite was accepted, is invalidated under the [Attestation lifecycle](statements.md#attestation). Its effect on each Assertion follows the [support projection](belief.md#support-projection), and an Assertion's identity never changes. Source entities, Occasions, testimony Attestations, and Assertions are untouched. A result derived only against the composite stays linked to it and never silently migrates to a member. Model-driven re-derivation is a later recorded Activity, never part of replay.

Truth-maintenance and provenance-semiring research support dependency-indexed invalidation. Limiting dependencies to revocable assumptions avoids general ATMS label growth. In practice a derivation's inputs cross zero or one composite, so the invalidated set follows from recorded inputs and no search is needed. The representation is a design synthesis with a tripwire: revisit it if the mean number of composites crossed per derivation ever exceeds one ([research lane](research/2026-07-24/lanes/identity-belief.md); [confidence](confidence.md#evidence-map)).

## Evidence and authority

Attribute overlap, biography similarity, and knowledge recitation do not support a merge on their own. Evidence keeps its kind and dependence lineage. Useful evidence includes connector continuity, authenticated challenge-response, operator confirmation, independent relational structure, and explicit self-identification. Relational structure raises the cost of impersonation without closing it: an attacker who first joins the same network, befriending the same people and attending the same events, can eventually forge relational evidence too.

At genesis the authorities are:

| Actor | May do |
|---|---|
| Conversational agent | Propose a candidate by recording an observation Activity with its evidence. |
| Connector challenge-response | Grant the clearance its registered policy defines, no more. |
| Operator | Accept at either clearance, change clearance or primary, reject, withdraw, or supersede. |

No off-turn authority accepts, rejects, or transitions a hypothesis at genesis. The conversational agent cannot accept a hypothesis, grant clearance, inspect unrestricted sibling history, or choose around the handle it was given.

Autonomous identity is the first capability planned after the successor lands ([evolution](evolution.md#deferred-capabilities)). It adds an off-turn authority that proposes and scores hypotheses and may accept at `recall` under a registered policy, together with the recall-restriction projection. `disclosure` still requires operator confirmation or challenge-response. It needs no new record kind. It reopens once evidence covers score calibration, overlap handling, adversarial resistance, one-handle behaviour, and thresholds for strengthening a tentative composite as corroboration accumulates and withdrawing it as its members' evidence diverges. Those thresholds are open empirical questions, because genuine same-person profiles also diverge.

## The operational wall

Identity resolution runs before conversational context is assembled. The agent receives one resolved handle per person under the current environment and writes to that handle, as [writes through a composite](#writes-through-a-composite) describes. It does not see sibling stubs, candidate scores, designated primaries, or conflict metadata. Operator diagnostics expose hypotheses, evidence, conflicts, and environments, and none of it reaches the conversational agent.

The wall keeps architectural metadata out of ordinary reasoning. It is not itself a security boundary: the audience resolver and the manifest-based restriction still enforce disclosure. The current system's sibling-history relay failures, including a measured 0.30 relay rate, motivate the one-handle surface. That the wall fully fixes the behaviour leak is a design inference, not an established result ([confidence](confidence.md#evidence-map)).

## Scenarios

| ID | Scenario | Expected result |
|---|---|---|
| `identity-overlap` | Candidates `{a, b}` and `{b, c}` exist; the operator accepts `{a, b}`, then tries to accept `{b, c}`. | Before acceptance, reads return three people. The second acceptance is rejected unless the same batch withdraws `{a, b}`. No `{a, b, c}` composite exists. |
| `identity-recall-restricted` | `{a, b}` is accepted at `recall`; `a` starts a conversation. | The turn's context manifest names no record from `b`. An operator diagnostic may render `b`'s content, and that Activity is never an input to a turn. |
| `identity-disclosure` | `{a, b}` is accepted at `disclosure`; `a` talks one-to-one, then in a group with `c`. | One-to-one, `b`'s `in_confidence` testimony may render for `a`, because `a` satisfies the teller-only principle through the composite. In the group, `c` is evaluated independently and the content is absent from the manifest. |
| `identity-unknown-audience` | A group includes a participant whose identity is unknown. | No cross-member content of any composite enters the group's context manifest. |
| `identity-clearance-upgrade` | `{a, b}` at `recall` is raised to `disclosure` by challenge-response. | A `clearance_changed` transition names the prior acceptance, and the composite ID is unchanged. Reads at later frontiers may render cross-member content into response-affecting context. |
| `identity-severance` | `{chat-a, forum-b}` is accepted as composite `r7`; a derivation Attestation takes inputs from both histories; later evidence shows two people. | Withdrawal restores both stub views in reads at later frontiers and invalidates that derivation Attestation. Testimony Attestations still name their source stubs. |
| `stub-joins-agent-entity` | The agent minted `person/rowan` before Rowan joined; Rowan's chat stub later appears, and the operator accepts `{person/rowan, stub}` with `person/rowan` as primary. | Neither entity is rewritten. Reads through the composite return one person. A proposal of `same_as` between `person/rowan` and an organisation entity is refused. |
| `composite-write-lands-on-primary` | While `r7` is accepted with `person/rowan` as primary, the agent writes a new Assertion about the person. | The Assertion's subject is `person/rowan`'s ULID. Its Attestation records the frontier, and the environment recomputed there names `r7` with that primary. The teller is the stub that spoke. |
| `severance-rehoming-review` | `r7` is withdrawn after three Assertions were written through it. | All three stay on the primary. A re-homing review item lists exactly those three for the operator. No Assertion moves until the operator supersedes it onto another member. |
| `identity-severance-no-migration` | A result derived only against composite `r7` exists when `r7` is withdrawn. | The result stays linked to `r7` and is not reassigned to either stub. |
| `identity-one-handle` | `{a, b}` is accepted at `disclosure`; the agent reads and writes about the person. | The agent sees one handle and no sibling stubs, scores, primaries, or conflict metadata. Its writes land on the primary member. |
| `identity-agent-cannot-accept` | The agent records a candidate and then attempts to accept it. | The candidate is recorded; the acceptance is rejected with a teachable error. |
| `identity-attribute-overlap-insufficient` | Two stubs share a birthday and employer, with no other evidence. | The candidate is recorded. An acceptance whose only evidence is attribute overlap is rejected under the initial policy. |
| `identity-stale-decision` | A connector challenge-response appends `clearance_changed` on `r7` while the operator withdraws `r7`; both name the same predecessor. | Whichever commits second is rejected as not naming the current head. A clearance is never applied to a withdrawn composite. |
| `identity-stub-rotation` | A platform rotates a user's identifier. | The connector appends an alias record; the original stub and its history remain. |
