import { createContext, useContext } from "react";

import type { Attachment } from "@zuihitsu/wire/types/Attachment.ts";

/// How a view reaches an attachment's bytes, or `null` when nothing here can.
///
/// The console is a read replica: it folds the log locally and asks no server for state. Bytes are not
/// state, though — an event log names an attachment by content address alone, and the bytes behind it
/// live wherever this viewer can reach them, which differs by frame. The live frame has an agent behind
/// it and reads them over `GET /blobs/{hash}`. The eval frame has no server at all, and resolves them
/// from the catalogue its package carries, minting an object URL per blob.
///
/// So a view asks for a URL and renders what it gets, rather than knowing which frame it is in. `null`
/// means this attachment's bytes are unreachable here — an address the catalogue does not carry, or a
/// frame with no source at all — and every treatment degrades to the announcement it can state from the
/// record alone.
///
/// A context rather than a prop because the fact is the frame's and only the leaves act on it — the
/// workspace and the transcript between them have nothing to say about it.
export interface BlobSource {
  /// The URL `attachment`'s bytes are reachable at, or `null` when they are not reachable here.
  urlFor(attachment: Attachment): string | null;
}

/// The source that reaches nothing: what a frame with no bytes behind it provides, and the default so
/// a view rendered outside any frame degrades rather than throwing.
export const NO_BLOBS: BlobSource = { urlFor: () => null };

export const BlobSourceContext = createContext<BlobSource>(NO_BLOBS);

export function useBlobSource(): BlobSource {
  return useContext(BlobSourceContext);
}

/// The source backed by a served agent: `GET /blobs/{hash}`, which is top-level and unauthenticated
/// because an `<img src>` cannot carry a bearer key and the content address is itself the capability.
/// `baseUrl` is the connection's — `""` for a same-origin console, an absolute origin for the dev
/// console proxying to the agent — so the URL is usable as an `src`, an `href`, or a `fetch` target
/// alike.
///
/// Every address resolves, whether or not the agent still holds the blob: a collected one answers
/// `404` and the image renders as broken rather than as the announcement. The agent is the authority
/// on what it has, and asking is how this frame finds out.
export function servedBlobs(baseUrl: string): BlobSource {
  return { urlFor: (attachment) => `${baseUrl}/blobs/${attachment.blob}` };
}
