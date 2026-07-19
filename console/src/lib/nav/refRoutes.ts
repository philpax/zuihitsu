import { type Mode, parsePath } from "./location.ts";

// Unprojecting a console URL back to the reference it points at — the frontend's own route knowledge,
// kept out of Rust. A console deep link is recognized by route matching, not token-syntax parsing: the
// URL is parsed with the standard URL API and run through the console's own route codec (`parsePath`).
// A State-view link routes by memory *handle* (`…/state/<handle>`); a Conversation-view link pins a
// moment in its `?turn=<id>` query. Either yields a value resolved and minted into a reference token
// elsewhere (the composer, the transcript chip); Rust never learns what a console URL looks like. This
// deliberately narrows to console routes — a foreign URL that merely carries a `?turn=` parameter is
// left as an ordinary link, not normalized.
//
// Route matching alone ignores host, so the send-time rewrite that *replaces* a URL with a token also
// gates on origin: it rewrites a deep link only when its origin is one the console owns (its own, or its
// configured backend), passed in by the caller. A foreign URL that happens to share the console's path
// shape is thus left as prose, never destroyed. Recognition (`stateHandleFromUrl`, `turnIdFromUrl`) stays
// origin-agnostic — the transcript's chip render resolves the named handle or moment against the folded
// graph and degrades when it names nothing, so it needs no origin gate; only the destructive rewrite
// does.

/// The memory handle a console State-view deep link names, or `null` when `url` is not one. Both URL
/// grammars are tried — the full console (`/live/state/…`, `/eval/<scenario>/<run>/state/…`) and the
/// embedded build (`/state/…` at the root) — since a pasted link may come from any deployment, and a
/// path only parses as a State route under the grammar that owns it. `parsePath` percent-decodes the
/// selection segment, so the returned handle is already decoded.
export function stateHandleFromUrl(url: string): string | null {
  const location = consoleLocation(url);
  if (
    location?.kind === "stream" &&
    location.stream.view === "state" &&
    location.stream.selection !== undefined
  ) {
    return location.stream.selection;
  }
  return null;
}

/// The turn id a console Conversation-view deep link pins, or `null` when `url` is not one — the turn
/// counterpart to [`stateHandleFromUrl`]. A conversation link carries the moment in its `?turn=<id>`
/// query (`/live/conversation?turn=<id>`, `/eval/<scenario>/<run>/conversation/<room>?turn=<id>`), so
/// this matches a stream Conversation route bearing that parameter, under either URL grammar. The value
/// is not validated as an id here; the caller's wasm constructor rejects a malformed one, leaving it
/// prose.
export function turnIdFromUrl(url: string): string | null {
  const location = consoleLocation(url);
  if (
    location?.kind === "stream" &&
    location.stream.view === "conversation" &&
    location.stream.search.turn !== undefined
  ) {
    return location.stream.search.turn;
  }
  return null;
}

/// Rewrite each console State-view deep link in `text` to the token `toToken` mints for its handle,
/// leaving the URL untouched when its origin is not one the console owns (see `origins`), it is not a
/// State link, or `toToken` declines it (an unresolved handle). URL recognition is the frontend's own
/// concern, so `text` is scanned for `http(s)` URLs here — not for token syntax; trailing sentence
/// punctuation is returned to the prose, so a link glued to a period keeps it.
export function rewriteStateUrls(
  text: string,
  toToken: (handle: string) => string | null,
  origins: readonly string[],
): string {
  return rewriteUrls(text, stateHandleFromUrl, toToken, origins);
}

/// Rewrite each console Conversation-view deep link in `text` to the token `toToken` mints for its
/// pinned turn id, leaving the URL untouched when its origin is not one the console owns (see `origins`),
/// it is not a conversation link, or `toToken` declines it (a malformed id). The turn counterpart to
/// [`rewriteStateUrls`].
export function rewriteTurnUrls(
  text: string,
  toToken: (id: string) => string | null,
  origins: readonly string[],
): string {
  return rewriteUrls(text, turnIdFromUrl, toToken, origins);
}

/// The console location a URL names, or `null` when it is not a valid URL or names no console route.
/// Both URL grammars are tried — the full console and the embedded build — since a pasted link may come
/// from any deployment, and a path only parses under the grammar that owns it.
function consoleLocation(url: string) {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return null;
  }
  const pathWithQuery = parsed.pathname + parsed.search;
  for (const mode of MODES) {
    const location = parsePath(pathWithQuery, mode);
    if (location !== null) return location;
  }
  return null;
}

/// Rewrite each `http(s)` URL in `text` that carries an origin the console owns (`origins`), that
/// `recognize` maps to a value, and that `toToken` mints a token for, leaving every other URL untouched.
/// The origin gate comes first, so a foreign URL sharing the console's path shape is never rewritten.
/// Trailing sentence punctuation is returned to the prose, so a link glued to a period keeps it.
function rewriteUrls(
  text: string,
  recognize: (url: string) => string | null,
  toToken: (value: string) => string | null,
  origins: readonly string[],
): string {
  return text.replace(URL_PATTERN, (match) => {
    const trimmed = match.replace(TRAILING_PUNCTUATION, "");
    if (!isOwnOrigin(trimmed, origins)) return match;
    const suffix = match.slice(trimmed.length);
    const value = recognize(trimmed);
    if (value === null) return match;
    const token = toToken(value);
    return token === null ? match : token + suffix;
  });
}

/// Whether `url`'s origin is one the console owns — its own page origin, or the configured backend origin
/// when it runs cross-origin against it. `false` for an unparseable URL or a foreign origin, so only a
/// URL the console genuinely serves is a candidate for rewriting.
function isOwnOrigin(url: string, origins: readonly string[]): boolean {
  try {
    return origins.includes(new URL(url).origin);
  } catch {
    return false;
  }
}

/// The URL grammars a console deep link can be written under, tried in turn.
const MODES: Mode[] = ["console", "embedded"];

/// An `http(s)` URL run, up to the first whitespace or URL-delimiter byte, so both reference kinds bound
/// a pasted URL identically.
const URL_PATTERN = /https?:\/\/[^\s<>"`{}|^\\]+/g;

/// Trailing sentence punctuation trimmed off a URL so it reads as prose, not part of the link.
const TRAILING_PUNCTUATION = /[.,;:!?)\]}'"]+$/;
