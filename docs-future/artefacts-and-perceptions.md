# Artefacts and perceptions

Artefact, ArtefactReference, and Perception separate immutable media, the act of sharing it, and a fallible observation of its content. The split applies to images, text files, audio, video, documents, tool outputs, and derived media.

## Permanent model

An Artefact has an algorithm-independent minted ID and identifies one immutable byte sequence. Immutable digest assertions bind that ID to an algorithm, algorithm version, digest, byte length, and verification result. Digest rotation appends another verified assertion; it does not mint a second Artefact or change its ID. A collision or conflicting length enters `quarantined` availability, denies byte use, and requires operator resolution. Physical deduplication and shared-byte retention use verified byte equivalence, not equal digest text. A digest identifies candidate content. It does not grant access.

Artefact has no semantic correction lifecycle because its bytes never change. Its availability projection folds verified storage locations, collision quarantine, key availability, and erasure records to `available`, `unavailable`, `quarantined`, or `erased`. Its retention projection folds live reference authority and explicit retention obligations. Neither projection changes byte identity.

An ArtefactReference records an Occasion-specific act of sharing. Its immutable source names the Artefact, supplier, original filename, ordered typed-content-part position, human caption or alt text, transmission principle, and retention authority. Identical bytes shared twice produce one Artefact and two references. Each reference retains independent provenance and audience. The append-only transition union is `reference_withdrawn`, `reference_retracted`, `reference_erased`, and `reference_authority_restored`. Each transition names authority, scope, reason, source head, and authorization decision. The fold is `authorised` initially. Withdrawal or retraction denies new consumption while retaining authorised audit history. `reference_authority_restored` may follow only a reversible withdrawal or retraction and can restore retention or access only when policy permits it. `reference_erased` leaves only the permitted tombstone and is terminal: restoration after erasure is rejected and cannot recreate payload or authority. A pending erasure may be cancelled through its authorization or hold decision before destructive execution. A later authorised sharing after completed erasure mints a new ArtefactReference with a new identity and authorization decision. A transition on one reference never changes another.

A Perception is a fallible observation produced by a named model or tool Activity. Every consumed typed content part records the ArtefactReference ID, selector ID and definition version, resolved Artefact ID, ordered position in the model/tool input, transformation pipeline and version, and the audience decision or access-record ID that authorised the bytes. Bare Artefact identity is never sufficient. The Perception also records model or tool identity, prompt or operation, implementation version, source head, output, and influence envelope. OCR and generated captions are Perceptions. They are not source utterances or participant testimony.

The Perception transition union is `perception_superseded`, `perception_retracted`, `perception_invalidated`, and `perception_erased`. A correction creates a new Perception and appends supersession; it never edits output. The fold is `current` initially, then `superseded`, `retracted`, `invalidated`, or `erased` according to the latest applicable transition. Historical audit may return retained non-current payloads only to an authorised operator. Conversational and retrieval projections return only current, authorised Perceptions. Reference withdrawal invalidates dependent Perceptions for ordinary use; erasure removes governed payloads and rebuilds dependants from surviving authorised inputs. Replay preserves tombstones and never reconstructs erased output.

An image-derived Assertion cites its Perception and the consumed ArtefactReference. Its source is the observing Activity. The system does not attribute the observation to the person who supplied the bytes.

A versioned Activity executes a transform and produces a separate Derivation whose typed output is a new Artefact. The Derivation records the input selector, output Artefact ID, pipeline and version, audience decision, influence envelope, and retention and erasure dependency. This rule applies to thumbnails, crops, EXIF stripping, PDF pages, audio segments, and video frames. OCR text and generated captions are Perceptions unless an operation also materialises output bytes, in which case the bytes are a derived Artefact and the observation remains a separate Perception.

Occasions preserve ordered typed content parts. Text and media can be interleaved. Adding a media type does not change Occasion meaning.

## Current attachment implementation

The current system stores attachment bytes in the content-addressed SQLite blob store. A `ConversationTurn` stores filename, media type, hash, length, and the `Image`, `Text`, or `Opaque` classification ([attachment type](../crates/core/src/attachment.rs#L18-L84), [event payload](../crates/core/src/event/payload.rs#L511-L538), [storage](../docs/events-and-storage.md#blob-store)). The Discord connector downloads and uploads attachment bytes and reports relay failures separately from later server/model rendering failures ([relay implementation](../platform-connectors/discord/src/bot/attachments/relay.rs#L91-L147), [relay tests](../platform-connectors/discord/src/bot/attachments/tests.rs#L225-L258), [server refusal fallback](../platform-connectors/discord/src/bot/process.rs#L85-L104), [connector behaviour](../platform-connectors/discord/README.md#behaviour)).

Supported images become image content parts in the model request. Text attachments are inlined within bounds. Opaque, missing, and unreadable stored attachments produce explicit server/model-render announcements ([attachment rendering](../src/agent/turn/attachments/mod.rs#L24-L147)). The serialized current request retains each consumed image's blob hash and MIME type while skipping base64 bytes, and `ModelCalled` records that request ([image part](../crates/core/src/model.rs#L50-L81), [request record](../crates/core/src/event/mod.rs#L527-L543), [recorder](../src/agent/turn/recording.rs#L130-L147)). It does not record a durable Perception, selector, reference-level authorization decision, or transform lineage. Replay can supply the same image again while replaying the turn if the blob remains available ([attachment replay test](../tests/agent/attachments.rs#L75-L109)). The console displays recorded attachments and loads images from the blob route ([component](../console/src/views/conversation/Attachments.tsx#L10-L27), [rendering](../console/src/views/conversation/Attachments.tsx#L109-L116), [image test](../console/src/views/conversation/Attachments.test.tsx#L62-L75), [route contract](../docs/events-and-storage.md#blob-store)).

Current semantic memory search does not retrieve an image by visual content. The agent also has no general API for historical blob inspection. This is established by the search implementation and exposed Lua/control API construction, not by inference from absent documentation ([memory search](../src/memory/search.rs), [memory API reference](../src/agent/lua/reference/memory.rs), [Lua tables](../src/agent/lua/tables/), [control API construction](../src/instance/control/actions.rs), [control API test](../src/http_server/tests/api/control.rs#L17-L39)).

The blob store is not reconstructed from the event log. Operators must back it up with the log ([storage contract](../docs/events-and-storage.md#blob-store)). These statements describe the current system only. The successor records references, perceptions, access, and erasure closure as first-class data.

## Source selectors

A selector is an immutable record with a minted selector ID, a registered definition ID and version, and an explicit target Artefact ID. Its canonical encoding is deterministic CBOR map `{0: selector_schema_version, 1: definition_id_and_version, 2: target_artefact_id, 3: typed_coordinates}`. Keys are unsigned integers in canonical order. Integers use shortest form, text is UTF-8, and the coordinate value is the definition's one canonical typed value. Version 1 uses schema value `1`, a combined stable definition/version string such as `whole/v1`, the minted Artefact ID string, and `null` for a whole-Artefact coordinate. No authenticated or semantic field lives outside these bytes. Equality is byte equality of that canonical encoding after schema validation. A selector always addresses the named original or derived Artefact. A transform never causes a selector to retarget implicitly.

| Selector variant | Version 1 semantics |
|---|---|
| `whole_artefact` | The complete target byte sequence. |
| `page_range` | Zero-based page indexes in a half-open `[start, end)` range over the page order produced by the named document-decoder definition and version. |
| `frame_range` | Zero-based decoded frame indexes in a half-open range under the named decoder and version. |
| `time_range` | A half-open interval in integer media-timescale ticks. The selector records the timescale and decoder version; it does not use floating-point seconds. |
| `spatial_region` | A half-open rectangle `(x, y, width, height)` in integer pixels of the orientation-normalised decoded raster. The selector records raster width, height, decoder version, and orientation-normalisation version. |
| `composite` | An ordered intersection of selectors with the same target and compatible decoder basis. Empty and cross-target composites are invalid. |

Bounds are validated at creation against mechanically known target metadata and the named decoder. Negative, reversed, empty where disallowed, overflowed, or out-of-range selectors are rejected and no Activity begins. A later decoder that changes page, frame, duration, orientation, or raster interpretation uses a new definition version and cannot reinterpret an old selector. Initial policy permits only `whole_artefact`; the other union variants and validation rules still exist at genesis so later activation does not rewrite history.

## Genesis substrate

The following data is required at successor genesis:

- durable ArtefactReference links;
- typed source locators and selectors;
- Activity input edges naming the authorising ArtefactReference, selector and definition version, resolved Artefact, ordered position, transformation pipeline and version, and audience decision or access record;
- Perception transitions and derived-Artefact Derivation record shapes;
- influence, audience, and transmission propagation;
- authenticated availability, retention, and erasure semantics;
- stable IDs and versions for every record and selector definition.

Replay does not invent a selector, source reference, model version, or influence edge that an old record omitted.

## Initial image policy

The initial policy preserves conversational image perception. Arrival alone creates no visual Assertion. If the agent writes durable image-derived memory, the write first records the applicable Perception and source lineage.

A query can return a prior Perception and its source reference without loading the bytes. Reinspection is an explicit audience-checked Activity. It records access and the model/tool call. A new observation creates a new Perception. It does not overwrite the previous observation.

## Gated extensions

Each extension has an independent evidence and privacy gate:

- controlled historical `inspect`;
- OCR on request or for a narrowly selected image class;
- generated captions and other Perceptions;
- page, frame, time, and region grounding;
- visual embeddings and cross-modal retrieval;
- automatic document and media ingestion;
- scene-graph extraction.

Scene-graph extraction remains furthest deferred because it creates a broad graph writer. Passing one extension does not enable another.

## Multimodal fixtures

| Scenario | Required result |
|---|---|
| Same bytes shared twice | One Artefact and two ArtefactReferences retain separate suppliers, audiences, and retention authority. |
| Conflicting human captions | Captions remain fields on their references. Neither becomes a Perception or Assertion automatically. |
| Image-only message | An Occasion contains an ArtefactReference and no utterance. Model consumption is an Activity. |
| OCR error | Original OCR Perception remains immutable. A corrected Perception and supersession lineage are appended. |
| Later reinspection | Audience is checked before bytes enter model context. The Activity and new Perception record versions and access. |
| Transformed crop | The crop is a derived Artefact with source selector and Derivation lineage. |
| Shared-blob erasure | One withdrawn reference loses authorization. Bytes remain while another authorized reference requires retention. |

Research supports content-addressed provenance and durable nondeterministic activities. The exact Artefact/Reference/Perception boundary, governed reinspection, selector representation, and multimodal erasure closure are design decisions that remain subject to the gates in [evolution](evolution.md) ([provenance research](research/2026-07-24/lanes/provenance-privacy.md), [welding research](research/2026-07-24/lanes/welding.md), [confidence evidence map](confidence.md#evidence-map)).