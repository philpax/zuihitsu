import type { LiveConnection } from "./live.ts";

/// The headers every control/platform request carries: a JSON content type, and the bearer key when
/// the connection has one (a loopback agent needs none). Shared by every `lib/` client so the auth
/// shape lives in one place.
export function authHeaders(connection: LiveConnection): HeadersInit {
  const headers: Record<string, string> = { "content-type": "application/json" };
  if (connection.key) headers.Authorization = `Bearer ${connection.key}`;
  return headers;
}

/// The origins a console-composed message's deep links may legitimately carry — the page's own origin
/// always, and the backend origin when the console runs cross-origin against it (the dev console against
/// `:7878`; the production console is same-origin, so `baseUrl` is `""` and only the page origin
/// applies). The send-time reference normalizer accepts a deep link only on one of these, so a foreign
/// URL that merely shares the console's route shape is left as prose rather than rewritten.
export function consoleOrigins(connection: LiveConnection): string[] {
  const origins = [window.location.origin];
  if (connection.baseUrl) {
    try {
      origins.push(new URL(connection.baseUrl).origin);
    } catch {
      /* a malformed baseUrl contributes no origin — the page origin still applies */
    }
  }
  return origins;
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
