import { type Mode, parsePath } from "./location.ts";

// Unprojecting a console URL back to the reference it points at — the frontend's own route knowledge,
// kept out of Rust. A console State-view deep link routes by memory *handle* (`…/state/<handle>`), so
// recognizing one is route matching, not token-syntax parsing: the URL is parsed with the standard URL
// API and run through the console's own route codec (`parsePath`). The handle it yields is then
// resolved to a memory and minted into its reference token elsewhere (the composer, the transcript
// chip); Rust never learns what a console URL looks like.

/// The memory handle a console State-view deep link names, or `null` when `url` is not one. Both URL
/// grammars are tried — the full console (`/live/state/…`, `/eval/<scenario>/<run>/state/…`) and the
/// embedded build (`/state/…` at the root) — since a pasted link may come from any deployment, and a
/// path only parses as a State route under the grammar that owns it. `parsePath` percent-decodes the
/// selection segment, so the returned handle is already decoded.
export function stateHandleFromUrl(url: string): string | null {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return null;
  }
  const pathWithQuery = parsed.pathname + parsed.search;
  for (const mode of MODES) {
    const location = parsePath(pathWithQuery, mode);
    if (
      location?.kind === "stream" &&
      location.stream.view === "state" &&
      location.stream.selection !== undefined
    ) {
      return location.stream.selection;
    }
  }
  return null;
}

/// Rewrite each console State-view deep link in `text` to the token `toToken` mints for its handle,
/// leaving the URL untouched when it is not a State link or `toToken` declines it (an unresolved
/// handle). URL recognition is the frontend's own concern, so `text` is scanned for `http(s)` URLs
/// here — not for token syntax; trailing sentence punctuation is returned to the prose, so a link glued
/// to a period keeps it.
export function rewriteStateUrls(text: string, toToken: (handle: string) => string | null): string {
  return text.replace(URL_PATTERN, (match) => {
    const trimmed = match.replace(TRAILING_PUNCTUATION, "");
    const suffix = match.slice(trimmed.length);
    const handle = stateHandleFromUrl(trimmed);
    if (handle === null) return match;
    const token = toToken(handle);
    return token === null ? match : token + suffix;
  });
}

/// The URL grammars a State deep link can be written under, tried in turn.
const MODES: Mode[] = ["console", "embedded"];

/// An `http(s)` URL run, up to the first whitespace or URL-delimiter byte — matching the boundary the
/// core turn-reference scanner uses so both reference kinds bound a pasted URL identically.
const URL_PATTERN = /https?:\/\/[^\s<>"`{}|^\\]+/g;

/// Trailing sentence punctuation trimmed off a URL so it reads as prose, not part of the link.
const TRAILING_PUNCTUATION = /[.,;:!?)\]}'"]+$/;
