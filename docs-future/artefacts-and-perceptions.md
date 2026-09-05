# Artefacts and perceptions

Artefact, ArtefactReference, and Perception separate immutable media, the act of sharing it, and a fallible observation of its content. The split applies to images, text files, audio, video, documents, tool outputs, and derived media.

## Permanent model

An Artefact has an algorithm-independent minted ID and identifies one immutable byte sequence. Immutable digest assertions bind that ID to an algorithm, algorithm version, digest, byte length, and verification result. Digest rotation appends another verified assertion; it does not mint a second Artefact or change its ID. A collision or conflicting length enters `quarantined` availability, denies byte use, and requires operator resolution. Physical deduplication and shared-byte retention use verified byte equivalence, not equal digest text. A digest identifies candidate content. It does not grant access.

Artefact has no semantic correction lifecycle because its bytes never change. Its availability projection folds verified storage locations, collision quarantine, and erasure records to `available`, `unavailable`, `quarantined`, or `erased`. Its retention projection is the set of its live authorised references; the bytes are deleted when that set becomes empty and never while a member survives. Neither projection changes byte identity.

An ArtefactReference records an Occasion-specific act of sharing. Its immutable source names the Artefact, supplier, original filename, media type, ordered typed-content-part position, and transmission principle. A live authorised reference is itself a retention authority for the bytes. Identical bytes shared twice produce one Artefact and two references. Each reference retains independent provenance and audience. The append-only transition union is `reference_withdrawn`, `reference_retracted`, `reference_erased`, and `reference_authority_restored`. Each transition names authority, scope, reason, source head, and authorization decision. The fold is `authorised` initially. Withdrawal or retraction denies new consumption while retaining authorised audit history. `reference_authority_restored` may follow only a reversible withdrawal or retraction and can restore retention or access only when policy permits it. `reference_erased` leaves only the permitted tombstone and is terminal: restoration after erasure is rejected and cannot recreate payload or authority. A pending erasure request may be declined by the operator before execution. A later authorised sharing after completed erasure mints a new ArtefactReference with a new identity and authorization decision. A transition on one reference never changes another.

A human caption or alt text is a text content part on the same Occasion, placed immediately after the reference part it describes and marked `caption_of` with that part's ID. The reference carries only mechanical metadata. A caption is participant-authored source text: it grounds an Attestation through an ordinary `text_part_span` locator, and it is never mechanically true because it accompanies the bytes.

A Perception is a fallible observation. It is the typed output of a Derivation produced by a named model or tool Activity, because an observation is computed and never a direct source. Every consumed typed content part records the ArtefactReference ID, selector and definition version, resolved Artefact ID, ordered position in the model/tool input, transformation pipeline and version, and the audience decision or access-record ID that authorised the bytes. Bare Artefact identity is never sufficient. The Perception also records model or tool identity, prompt or operation, implementation version, source head, output, and influence envelope. OCR and generated captions are Perceptions. They are not source utterances or participant testimony.

The Perception transition union is `perception_superseded`, `perception_retracted`, `perception_invalidated`, and `perception_erased`. A correction creates a new Perception and appends supersession; it never edits output. The fold is `current` initially, then `superseded`, `retracted`, `invalidated`, or `erased` according to the latest applicable transition. Historical audit may return retained non-current payloads only to an authorised operator. Conversational and retrieval projections return only current, authorised Perceptions. Reference withdrawal invalidates dependent Perceptions for ordinary use; erasure removes governed payloads and rebuilds dependants from surviving authorised inputs. Replay preserves tombstones and never reconstructs erased output.

An image-derived Assertion cites its Perception and the consumed ArtefactReference. Its source is the observing Activity. The system does not attribute the observation to the person who supplied the bytes.

A versioned Activity executes a transform and produces a separate Derivation whose typed output is a new Artefact. The Derivation records the input selector, output Artefact ID, pipeline and version, audience decision, influence envelope, and retention and erasure dependency. This rule applies to thumbnails, crops, EXIF stripping, PDF pages, audio segments, and video frames. OCR text and generated captions are Perceptions unless an operation also materialises output bytes, in which case the bytes are a derived Artefact and the observation remains a separate Perception.

Occasions preserve ordered typed content parts. Text and media can be interleaved. Adding a media type does not change Occasion meaning.

## Source selectors

A selector is an immutable content-keyed value with no minted ID: its identity is its canonical encoding, and a record that names a selector carries that encoding or its digest. It has a registered definition ID and version and an explicit target Artefact ID. Its canonical encoding is deterministic CBOR map `{0: selector_schema_version, 1: definition_id_and_version, 2: target_artefact_id, 3: typed_coordinates}`. Keys are unsigned integers in canonical order. Integers use shortest form, text is UTF-8, and the coordinate value is the definition's one canonical typed value. Version 1 uses schema value `1`, a combined stable definition/version string such as `whole_artefact/v1`, the minted Artefact ID string, and `null` for a whole-Artefact coordinate. No authenticated or semantic field lives outside these bytes. Equality is byte equality of that canonical encoding after schema validation; two records that carry equal bytes name one selector. A selector always addresses the named original or derived Artefact. A transform never causes a selector to retarget implicitly.

| Selector variant | Version 1 semantics |
|---|---|
| `whole_artefact` | The complete target byte sequence. |
| `page_range` | Zero-based page indexes in a half-open `[start, end)` range over the page order produced by the named document-decoder definition and version. |
| `frame_range` | Zero-based decoded frame indexes in a half-open range under the named decoder and version. |
| `time_range` | A half-open interval in integer media-timescale ticks. The selector records the timescale and decoder version; it does not use floating-point seconds. |
| `spatial_region` | A half-open rectangle `(x, y, width, height)` in integer pixels of the orientation-normalised decoded raster. The selector records raster width, height, decoder version, and orientation-normalisation version. |
| `byte_range` | A half-open `[start, end)` range of byte offsets over the raw target byte sequence. It names no decoder and applies to any Artefact. |
| `composite` | An ordered intersection of selectors with the same target and compatible decoder basis. Empty and cross-target composites are invalid. |

Bounds are validated at creation against mechanically known target metadata and the named decoder. Negative, reversed, empty where disallowed, overflowed, or out-of-range selectors are rejected and no Activity begins. A later decoder that changes page, frame, duration, orientation, or raster interpretation uses a new definition version and cannot reinterpret an old selector. Initial policy permits only `whole_artefact`; the other union variants and validation rules still exist at genesis so later activation does not rewrite history.

## Genesis substrate

The following data is required at successor genesis:

- durable ArtefactReference links;
- typed source locators and selectors;
- Activity input edges naming the authorising ArtefactReference, selector and definition version, resolved Artefact, ordered position, transformation pipeline and version, and audience decision or access record;
- Perception transitions and derived-Artefact Derivation record shapes;
- influence, audience, and transmission propagation;
- availability, reference-set retention, and erasure semantics;
- stable IDs and versions for every record and selector definition.

Replay does not invent a selector, source reference, model version, or influence edge that an old record omitted.

## Initial image policy

The initial policy preserves conversational image perception. Arrival alone creates no visual Assertion. If the agent writes durable image-derived memory, the write first records the applicable Perception and source lineage.

A query can return a prior Perception and its source reference without loading the bytes. Reinspection is an explicit audience-checked Activity. It records access and the model/tool call. A new observation creates a new Perception. It does not overwrite the previous observation.

## Activation-gate capabilities

Each listed capability has its own `activation_gate` record, independent evidence IDs, privacy oracle, disabled-behaviour oracle, and additive-seam reference:

- `capability:historical-reinspection`: controlled historical `inspect`;
- `capability:ocr`: OCR on request or for a narrowly selected image class;
- `capability:generated-captions`: generated captions and other Perceptions;
- `capability:region-grounding`: page, frame, time, and region grounding;
- `capability:visual-retrieval`: visual embeddings and cross-modal retrieval;
- `capability:bulk-ingestion`: automatic document and media ingestion, with `stage:8` as its operational prerequisite;
- `capability:scene-graph-writer`: scene-graph extraction.

`capability:scene-graph-writer` is an independent `activation_gate`, currently not selected; its broad graph-writer risk requires its own evidence, disabled-behaviour oracle, additive-seam reference, and activation decision. Passing one capability gate does not enable another.

## Multimodal fixtures

| Scenario | Required result |
|---|---|
| Same bytes shared twice | One Artefact and two ArtefactReferences retain separate suppliers and audiences; each is an independent retention authority for the bytes. |
| Conflicting human captions | Each caption is a text part marked `caption_of` on its own Occasion. Neither becomes a Perception or Assertion automatically. |
| Image-only message | An Occasion contains an ArtefactReference and no utterance. Model consumption is an Activity. |
| OCR error | Original OCR Perception remains immutable. A corrected Perception and supersession lineage are appended. |
| Later reinspection | Audience is checked before bytes enter model context. The Activity and new Perception record versions and access. |
| Transformed crop | The crop is a derived Artefact with source selector and Derivation lineage. |
| Shared-reference erasure | One erased reference loses authorisation. Managed live bytes remain while another authorised reference requires retention. A terminally erased reference cannot restore authority. |

Research supports content-addressed provenance and durable nondeterministic activities. The Artefact/Reference/Perception boundary and selector and erasure substrate are owned by `stage:3` and `stage:6`; historical reinspection and multimodal interpretation are separate activation-gate capabilities ([provenance research](research/2026-07-24/lanes/provenance-privacy.md), [welding research](research/2026-07-24/lanes/welding.md), [confidence evidence map](confidence.md#evidence-map)).