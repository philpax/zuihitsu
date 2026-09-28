# Artefacts and perceptions

Artefact, ArtefactReference, and Perception separate immutable media, the act of sharing it, and a fallible observation of its content. The split applies to images, text files, documents, audio, video, tool outputs, and derived media. [The object model](statements.md) defines these objects and their lifecycles. This chapter specifies how they behave: identity and deduplication, selectors, text extraction for documents, perception, and the genesis image policy.

## Artefact identity

An Artefact is a minted ULID for one immutable byte sequence. The ID is not derived from the content. The content digest, byte length, and other mechanically observed byte metadata live in the Artefact's erasable payload. The deduplication index is built from payloads: when new bytes arrive, the store looks up their digest in the index and reuses the existing Artefact ID on a match. An erased Artefact's payload is gone, so its digest leaves the index and its tombstone keeps only the ID. A later upload of the same bytes mints a new Artefact.

The ID is minted rather than content-addressed for erasure. If the ID were a content hash, an erased file's tombstone would still let anyone confirm a guessed file by hashing it and finding the tombstone. A digest identifies candidate content for deduplication. It never grants access.

An Artefact has no correction lifecycle because its bytes never change. Its availability and retention folds are defined in [the object model](statements.md#artefact-and-artefactreference). Neither changes the ID.

## References and captions

An ArtefactReference records one sharing act on one Occasion. It names the Artefact, the supplier, the original filename, the media type, the part's position in the Occasion's ordered content, and the transmission principle. Identical bytes shared twice produce one Artefact and two references, and each reference keeps its own supplier, audience, and [lifecycle](statements.md#legal-transitions). Withdrawal and retraction deny new consumption and are reversible. A later sharing of the same bytes mints a new reference, and a transition on one reference never changes another.

Access to bytes always goes through a reference. Bare Artefact identity is never sufficient, because the same bytes can be authorised for one audience through one reference and hidden from another through a second.

A human caption or alt text is a text part on the same Occasion, placed after the reference part it describes and marked `caption_of` that part. The reference carries only mechanical metadata. A caption is participant-authored source text: it can ground an Attestation through an ordinary text locator, and it is never mechanically true because it accompanies the bytes.

## Selectors

A selector addresses part of one named Artefact. It has no minted ID: it is a content-keyed value, and two selectors are equal when their canonical encodings are byte-equal. The implementation chooses the encoding. Each selector names its variant, its definition version, and its target Artefact ID. A selector never retargets implicitly: a transform produces a new Artefact, and a selector over it names that Artefact.

The genesis variants are:

| Variant | Semantics |
|---|---|
| `whole_artefact` | The complete target byte sequence. |
| `page_range` | Zero-based page indexes in a half-open `[start, end)` range, in the page order produced by a named and versioned document decoder. |
| `text_span` | A half-open `[start, end)` range of Unicode scalar offsets over a derived text Artefact produced by text extraction. |

Bounds are validated at creation against the target's known metadata and the named decoder. A negative, reversed, empty, overflowing, or out-of-range selector is rejected, and no Activity begins. A decoder change that alters page order or text extraction registers a new definition version. It cannot reinterpret a selector written under the old one.

These three variants are required at genesis because the grounding they record cannot be recovered later. A claim written from a book the agent read, without a page or span locator, can only be regrounded by reading the book again.

## Reading a document

Text extraction is a recorded tool Activity over an ArtefactReference to a document and a `whole_artefact` selector. It runs once per source Artefact and extractor version, whichever reference first consumes it. Its output is a derived text Artefact (the extracted UTF-8 text) and a page map. The page map is an ordered list with one entry per page index under the named decoder version: the scalar offset in the derived text where that page's text begins, and the page's printed label when the decoder exposes one. The page map lives in the derived Artefact's erasable payload. A later read of the same Artefact under the same extractor version reuses the existing derived Artefact, through any reference.

The derived text Artefact has no references of its own. Its readability always follows the consuming reference, never the reference that first produced it, and its retention follows the source Artefact's retention. When the source is erased, dependant invalidation deletes the derived text and its page map.

A book the agent reads over several turns follows this path:

1. A participant shares a PDF. The Occasion holds an ArtefactReference to the book's Artefact.
2. The agent calls `inspect` with a `page_range` or `text_span` selector. The first call triggers extraction. `inspect` checks the reference's audience, renders the requested text, and appends the derived Artefact ID and the selector to the context manifest.
3. In a later turn the agent writes a claim from that reading. The claim's source locator names the book's ArtefactReference and a `text_span` over the derived text. The Attestation is a `derivation` over the reading Activity, because interpreting the text is computed: its inputs are the reference, the derived text Artefact, and the span. Neither the book's author nor the person who shared it is a teller. A claim the book makes is reported speech, so it is always `quoted` and reads render it as "the book says X" with its Artefact source, never as a flat claim ([teller binding](write-surface.md#teller-binding)).
4. A reader resolves the span to pages through the page map: the cited pages are the contiguous range of page indexes whose text intersects the span. The result is a `page_range` over the original book under the same decoder version, displayed with printed labels where the page map has them.

The [grounding critic](verified-write.md#hard-critics) requires containment in a rendered span, so after reading pages 40 to 43, a span that runs into page 44 is rejected. A claim drawn from two separate reads cites two locators.

For a scanned document the extractor is OCR, and the extracted text is a fallible observation. OCR is deferred (see below). A plain UTF-8 text file needs no decoder: when the extracted text equals the source bytes, deduplication makes the derived text Artefact the source Artefact itself, and the page map is empty.

## Perceptions

A Perception is the fallible output of a model or tool Activity over an ArtefactReference and a selector. It is never testimony. It records its producing Activity, the consumed reference, selector, and resolved Artefact, the model or tool identity and version, the prompt or operation, and the output. The producing Activity's context manifest records what else was rendered. OCR text and generated captions are Perceptions, not source utterances.

An image-derived Assertion cites its Perception and the consumed ArtefactReference. Its source is the observing Activity, never the person who supplied the bytes.

The Perception lifecycle, including unusability while the consumed reference is not `authorised` as a read-time projection, is in [the object model](statements.md#perception). Output is never edited. Conversational and retrieval reads return only current, usable Perceptions. An authorised operator can read retained non-current payloads for audit. Erasure removes payloads and rebuilds dependants from surviving authorised inputs.

A transform that materialises bytes (a thumbnail, EXIF stripping, a rendered page, an extracted text file) is a tool Activity whose output is a derived Artefact. The derived Artefact records its producing Activity and typed inputs directly. When an operation both materialises bytes and observes content, the bytes are a derived Artefact and the observation is a separate Perception.

## Genesis image policy

The genesis policy preserves conversational image perception. The model sees an image shared in the current turn through an ordinary recorded Activity. Arrival alone creates no Assertion. When the agent writes durable image-derived memory, the write first records the Perception and its source lineage.

A query can return a prior Perception and its source reference without loading the bytes. A later look at the bytes is an explicit `inspect` ([query surface](query-surface.md#source-retrieval-and-reinspection)): it checks the audience before bytes enter a model context, records the Activity, and creates a new Perception. It never overwrites the previous one.

## Deferred capabilities

Each item is additive. None requires rewriting existing records. [Evolution](program/evolution.md#deferred-capabilities) holds the reopen conditions.

- The `spatial_region`, `frame_range`, `time_range`, and `byte_range` selector variants, for region, video-frame, media-time, and raw-byte grounding. Each needs its own decoder basis: raster orientation and dimensions, frame decoder, or integer timescale.
- OCR, including extraction from scanned documents.
- Generated captions and other generated Perceptions outside the current turn.
- Visual embeddings and cross-modal retrieval.
- Automatic bulk ingestion of documents and media ([memory typology](memory-typology.md#conversational-artefacts-are-not-bulk-ingestion)).
- Scene-graph extraction, which is a broad graph writer and needs its own evidence.

Enabling one deferred capability never enables another.

## Evidence

Research supports content-addressed provenance and durable records of nondeterministic Activities ([provenance research](research/2026-07-24/lanes/provenance-privacy.md), [welding research](research/2026-07-24/lanes/welding.md)). The Artefact, Reference, and Perception boundary, the page map, and the selector set are design synthesis ([confidence evidence map](program/confidence.md#evidence-map)).

## Scenarios

| ID | Scenario | Expected result |
|---|---|---|
| `same-bytes-two-references` | Two participants share identical bytes to different audiences. | One Artefact and two references. Each keeps its supplier and audience and is an independent retention authority. Neither audience consumes through the other's reference. |
| `conflicting-captions` | The same image carries different human captions on two Occasions. | Each caption is a text part marked `caption_of` on its own Occasion. Neither becomes a Perception or Assertion automatically. |
| `caption-perception-conflict` | A participant captions an image "Pepper at the harbour"; the model sees an indoor room. | The caption stays source text and the Perception stays a model observation, not attributed to the participant. No `contradiction_detected` mark is appended. |
| `image-only-occasion` | A participant sends an image with no text. | An Occasion with one reference part and no text part is valid. Model consumption is an Activity. No Assertion follows from arrival. |
| `perception-correction` | An OCR Perception reads "R0WAN", and a later one reads "ROWAN". | The original stays immutable, and the new Perception supersedes it. After the source reference is erased, neither text returns through search, snapshot, export, or retry context. |
| `later-reinspection` | The agent inspects an image shared in an earlier Occasion. | The audience is checked before bytes enter context. A new Perception is recorded, and the earlier one is unchanged. |
| `derived-artefact-lineage` | A thumbnail is generated from a shared image. | The thumbnail is a derived Artefact that names its producing Activity and its input reference and selector. |
| `withdrawn-reference-retains` | The only reference to some bytes is withdrawn. | The bytes are kept, because a withdrawn reference can be restored. They are deleted only when the reference is erased. |
| `shared-reference-erasure` | One of two references to the same bytes is erased. | The erased reference loses authority and cannot be restored. The bytes remain while the other reference is not erased. |
| `erased-artefact-reupload` | Bytes whose Artefact was erased are uploaded again. | The digest is absent from the deduplication index, so a new Artefact ID is minted. The tombstone does not reveal that the bytes match. |
| `page-span-citation` | The agent reads pages 40 to 43 of a shared book and later writes a claim from them. | The claim is a `derivation` Attestation over the reading Activity and cites a `text_span` over the derived text. Resolution through the page map yields `page_range [40, 44)` under the same decoder version. |
| `book-claim-contained-span` | After reading pages 40 to 43, the agent cites a span that starts on page 43 and ends on page 44. | The grounding critic rejects it: the span overlaps the rendered text but is not contained in it. |
| `book-claim-quoted` | The agent records that the book says a harbour was built in 1850. | The Assertion is `quoted`, its Attestation is a derivation, and no teller is named. A read renders it as what the book says, with the Artefact source. |
| `extraction-reused` | The agent inspects a second chapter of the same book. | No new extraction runs. The existing derived text Artefact and page map serve the read. |
| `extraction-reuse-follows-consumer` | Rowan and Quinn share the same PDF by references A and B, text was extracted through A, and A is then withdrawn. | A read through B uses the existing derived text and is checked against B's audience. A read through A is denied. |
| `withdrawn-reference-perception` | A reference with a Perception is withdrawn, then restored. | While withdrawn, reads omit the Perception and no `perception_invalidated` is appended. After restoration, reads return it. |
| `selector-out-of-bounds` | A `page_range` ends past the decoder's page count. | The selector is rejected and no Activity begins. |
| `deferred-selector-rejected` | A write names a `spatial_region` selector at genesis. | The write is rejected with a teachable error naming the genesis variants. |
| `book-source-erased` | The book's only reference is erased. | The derived text Artefact and page map are deleted. Claims citing its spans are invalidated through their input edges. |
| `initial-perception` | A caller authorised through reference A has a model describe the bytes. | The Perception records reference A, the selector, the resolved Artefact, and the model version. A later A read returns it without reloading bytes. A read for an audience cleared only by reference B returns no Perception and an unchanged ranking. |
| `denied-reinspection` | A caller excluded by reference B asks to inspect the same bytes through B. | No bytes are loaded and no model call, Perception, or index row is created; nothing in the reply reveals the denial detail. |
| `authorised-reinspection` | An authorised caller explicitly reinspects through A under a newer model. | A new Perception is created and may supersede the old one; unauthorised audiences see neither observation nor a changed ranking. |
| `reference-lifecycle` | A reference is withdrawn, restored, and erased. | Restoration returns it to `authorised`; erasure is terminal; a later share mints a new reference. |
