import { createContext, useContext } from "react";

/// How a view reaches an attachment's bytes, which differs by frame: the live frame has an agent
/// behind it, the eval frame has only the catalogue its package carries. A view asks for a URL and
/// renders what it gets. `null` means the bytes are unreachable here, and every treatment degrades to
/// the announcement the record alone states.
export interface BlobSource {
  /// The URL these bytes are reachable at, by content address alone — so an `Attachment` and a
  /// prompt's `ImagePart` resolve through the same call.
  urlFor(addressed: { blob: string }): string | null;
}

/// The source that reaches nothing, and the default so a view outside any frame degrades rather than
/// throwing.
export const NO_BLOBS: BlobSource = { urlFor: () => null };

export const BlobSourceContext = createContext<BlobSource>(NO_BLOBS);

export function useBlobSource(): BlobSource {
  return useContext(BlobSourceContext);
}

/// The source backed by a served agent. `baseUrl` is the connection's — `""` same-origin, an absolute
/// origin for the dev console — so the URL works as an `src`, an `href`, or a `fetch` target.
///
/// Every address resolves whether or not the agent still holds the blob: a collected one answers
/// `404`. The agent is the authority on what it has, and asking is how this frame finds out.
export function servedBlobs(baseUrl: string): BlobSource {
  return { urlFor: ({ blob }) => `${baseUrl}/blobs/${blob}` };
}
