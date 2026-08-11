import type { BlobHash } from "@zuihitsu/wire/types/BlobHash.ts";
import type { LiveConnection } from "./live.ts";
import { errorMessage } from "./http.ts";

/// The media type an upload whose file carries none is stored under, matching the server's own
/// fallback: "some bytes", which is all the browser told us.
export const OCTET_STREAM = "application/octet-stream";

/// Where an attachment's bytes are served: `GET /blobs/{hash}` on the agent, which is top-level and
/// unauthenticated because an `<img src>` cannot carry a bearer key and the content address is itself
/// the capability. `baseUrl` is the connection's — `""` for a same-origin console, an absolute origin
/// for the dev console proxying to the agent — so the URL is usable as an `src`, an `href`, or a
/// `fetch` target alike.
export function blobUrl(baseUrl: string, hash: BlobHash): string {
  return `${baseUrl}/blobs/${hash}`;
}

/// Store a file's bytes and return their content address, the console acting as a platform connector
/// (`POST /platform/blobs`). The body is the bytes themselves and the `Content-Type` is the media
/// type they are stored under, so there is no envelope to build. Idempotent by construction: the same
/// bytes always answer with the same address.
///
/// A message may only name a blob the store already holds, so an attachment is uploaded as it is
/// added to the composer rather than at send — a failure then surfaces while the sender is still
/// typing, not after they have committed to the message.
export async function uploadBlob(connection: LiveConnection, file: File): Promise<BlobHash> {
  // Not `authHeaders`: this body is the file, not JSON, and the content type is what the bytes are
  // stored under.
  const headers: Record<string, string> = { "content-type": file.type || OCTET_STREAM };
  if (connection.key) headers.Authorization = `Bearer ${connection.key}`;
  const response = await fetch(`${connection.baseUrl}/platform/blobs`, {
    method: "POST",
    headers,
    body: file,
  });
  if (!response.ok) throw new Error(await errorMessage(response));
  const body = (await response.json()) as { hash: BlobHash };
  return body.hash;
}
