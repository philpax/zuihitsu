import { type ViewId, SELECTION_VIEWS, asStreamViewId, asViewId } from "./streamViews.ts";

// ---- The location model ----

/// The console's location, modelled as a typed value rather than a tree of route positions. This is
/// the single source of truth for routing: [`parsePath`] decodes a URL into it, [`buildPath`] encodes
/// it back (the two are a bijection over canonical URLs — see `location.test.ts`), and every hook,
/// link, and screen reads or constructs one. A stream location holds its [`StreamFrame`] as a field,
/// not a path prefix — which dissolves the "route-agnostic shared view" problem: a view rendered under
/// the eval run, the live agent, or the embedded build reads its location, keeps the `frame` it finds,
/// and changes only the `stream`, so its links carry the frame forward without the view ever naming it.
export type AppLocation =
  | { kind: "landing" }
  | { kind: "trends" }
  | { kind: "evalOverview" }
  | { kind: "stream"; frame: StreamFrame; stream: StreamView };

/// The frame a stream location lives in — the value a stream view carries forward when it navigates.
/// `evalRun` names its scenario and run; `live` and `embedded` carry nothing beyond their kind.
export type StreamFrame =
  | { kind: "evalRun"; scenario: string; run: number }
  | { kind: "live" }
  | { kind: "embedded" };

/// A stream frame's inner location: which view, its navigational selection (present only for the
/// selection-bearing views), and the search that rides across the views.
export interface StreamView {
  view: ViewId;
  /// The open memory (State), room (Conversation), or settings tab (Settings) — the encoded trailing
  /// path segment. Absent for the views that carry no selection, and for a selection view at its
  /// default (a bare `/state`).
  selection?: string;
  search: StreamSearch;
}

/// The query state shared across every stream view: the timeline cursor, a deep-linked turn's
/// highlight, the Events view's memory pin, and the Relations view's graph filters.
export interface StreamSearch {
  seq?: number;
  turn?: string;
  focus?: string;
  relations?: string;
  sameAs?: string;
  expand?: string;
}

/// Which grammar the URL follows: the full console (landing, eval, trends, live) or the embedded agent
/// build (the stream at the root). Fixed for a build by `window.__APP_MODE__`.
export type Mode = "console" | "embedded";

// ---- Decomposition and constructors ----

/// The frame and stream of a stream location, or `null` for a non-stream one — the pair a view reads
/// to keep its frame while changing its view or selection.
export function streamPartsOf(
  location: AppLocation,
): { frame: StreamFrame; stream: StreamView } | null {
  return location.kind === "stream" ? { frame: location.frame, stream: location.stream } : null;
}

/// A stream location from a frame and a (possibly edited) stream view — the target a view builds to
/// preserve its current frame.
export function streamLocation(frame: StreamFrame, stream: StreamView): AppLocation {
  return { kind: "stream", frame, stream };
}

/// A run opened at a view (the conversation by default), for the eval overview and rail links that
/// point at a *different* run than the current one — where there is no current frame to carry forward.
export function evalRunLocation(
  scenario: string,
  run: number,
  view: ViewId = "conversation",
): AppLocation {
  return streamLocation({ kind: "evalRun", scenario, run }, { view, search: {} });
}

// ---- The path codec ----

/// Encode a location to a canonical URL path (pathname plus any query). Each frame owns its prefix, so
/// this needs no `Mode` — the location itself says which grammar it belongs to.
export function buildPath(location: AppLocation): string {
  switch (location.kind) {
    case "landing":
      return "/";
    case "trends":
      return "/trends";
    case "evalOverview":
      return "/eval";
    case "stream":
      return framePrefix(location.frame) + streamSuffix(location.stream);
  }
}

/// Decode a URL path (pathname plus optional `?query`) into a location, or `null` when it names no
/// reachable place — a typo, a stale link, or a malformed deep URL — which the caller recovers from.
export function parsePath(path: string, mode: Mode): AppLocation | null {
  const queryAt = path.indexOf("?");
  const pathname = queryAt === -1 ? path : path.slice(0, queryAt);
  const query = queryAt === -1 ? "" : path.slice(queryAt + 1);
  const segments = pathname.split("/").filter(Boolean).map(decodeURIComponent);
  const search = parseSearch(query);

  if (mode === "embedded") {
    // The agent's own view is the whole app: the stream sits at the root, no landing or eval around it.
    const stream = parseStreamSegments(segments, search, asViewId);
    return stream ? streamLocation({ kind: "embedded" }, stream) : null;
  }

  if (segments.length === 0) return { kind: "landing" };
  if (segments[0] === "trends" && segments.length === 1) return { kind: "trends" };

  if (segments[0] === "eval") {
    if (segments.length === 1) return { kind: "evalOverview" };
    // /eval/<scenario>/<run>/<view>(/<selection>) — the eval frame carries only the stream views.
    const [, scenario, runText, ...rest] = segments;
    const run = Number(runText);
    if (scenario === undefined || runText === undefined || !Number.isInteger(run) || run < 0) {
      return null;
    }
    const stream = parseStreamSegments(rest, search, asStreamViewId);
    return stream ? streamLocation({ kind: "evalRun", scenario, run }, stream) : null;
  }

  if (segments[0] === "live") {
    const stream = parseStreamSegments(segments.slice(1), search, asViewId);
    return stream ? streamLocation({ kind: "live" }, stream) : null;
  }

  return null;
}

/// The URL prefix a frame owns, before its stream suffix.
function framePrefix(frame: StreamFrame): string {
  switch (frame.kind) {
    case "evalRun":
      return `/eval/${encodeURIComponent(frame.scenario)}/${frame.run}`;
    case "live":
      return "/live";
    case "embedded":
      return "";
  }
}

/// The stream view's URL suffix: `/<view>` and, for a selection view opened on one, `/<selection>`,
/// then any query.
function streamSuffix(stream: StreamView): string {
  const selection =
    stream.selection !== undefined ? `/${encodeURIComponent(stream.selection)}` : "";
  return `/${stream.view}${selection}${buildSearch(stream.search)}`;
}

/// Decode a frame's trailing segments — `[]`, `[view]`, or `[view, selection]` — into a [`StreamView`],
/// validating the view through `asView` (the eval frame admits only stream views; the live and
/// embedded frames admit the agent-only views too) and gating the selection on the view. `[]` defaults
/// to the conversation, the payoff view.
function parseStreamSegments(
  segments: string[],
  search: StreamSearch,
  asView: (value: string | undefined) => ViewId | undefined,
): StreamView | null {
  if (segments.length === 0) return { view: "conversation", search };
  if (segments.length > 2) return null;

  const [viewText, selection] = segments;
  const view = asView(viewText);
  if (view === undefined) return null;

  if (selection === undefined) return { view, search };
  if (!SELECTION_VIEWS.has(view)) return null;
  return { view, selection, search };
}

// ---- The search codec ----

/// The string-valued search keys, in a stable canonical order. `seq` is handled apart, since it
/// coerces to a number.
const SEARCH_STRINGS = ["turn", "focus", "relations", "sameAs", "expand"] as const;

/// Parse a raw query string into the typed search — coercing the cursor to a number, dropping blank or
/// absent keys, so the parsed object round-trips with what [`buildSearch`] emits.
function parseSearch(query: string): StreamSearch {
  const params = new URLSearchParams(query);
  const search: StreamSearch = {};
  const seq = params.get("seq");
  if (seq !== null && seq !== "") {
    // A seq is an event sequence number, so hold it to the same discipline as the run index — a
    // non-negative integer — rather than admitting `?seq=1.5` or `?seq=-3`.
    const value = Number(seq);
    if (Number.isInteger(value) && value >= 0) search.seq = value;
  }
  for (const key of SEARCH_STRINGS) {
    const value = params.get(key);
    if (value !== null && value !== "") search[key] = value;
  }
  return search;
}

/// Serialise the search back to a query string (with a leading `?`), or `""` when empty. A stable key
/// order keeps the URL canonical, though the parsed object is order-independent.
function buildSearch(search: StreamSearch): string {
  const params = new URLSearchParams();
  if (search.seq !== undefined) params.set("seq", String(search.seq));
  for (const key of SEARCH_STRINGS) {
    const value = search[key];
    if (value !== undefined && value !== "") params.set(key, value);
  }
  const query = params.toString();
  return query ? `?${query}` : "";
}
