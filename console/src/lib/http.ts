import type { LiveConnection } from "./live.ts";

/// The headers every control/platform request carries: a JSON content type, and the bearer key when
/// the connection has one (a loopback agent needs none). Shared by every `lib/` client so the auth
/// shape lives in one place.
export function authHeaders(connection: LiveConnection): HeadersInit {
  const headers: Record<string, string> = { "content-type": "application/json" };
  if (connection.key) headers.Authorization = `Bearer ${connection.key}`;
  return headers;
}

/// Render a failed response as a message. The server sends a structured `{ error }` body (the Rust
/// `ErrorBody`), so prefer that; fall back to the status line when the body is absent or not JSON.
export async function errorMessage(response: Response): Promise<string> {
  try {
    const body = (await response.json()) as { error?: string };
    if (body.error) return body.error;
  } catch {
    /* fall through to the status line */
  }
  return `the agent answered ${response.status} ${response.statusText}`;
}
