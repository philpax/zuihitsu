import {
  type AppLocation,
  type StreamFrame,
  type StreamSearch,
  type StreamView,
  streamLocation,
  streamPartsOf,
} from "./location.ts";
import type { ViewId } from "./streamViews.ts";
import { type Navigate, useLocation, useNavigate } from "./historyContext.ts";

/// The stream views' window onto routing. Because the frame is a field of the [`AppLocation`] — not a
/// path prefix — a view rendered under the eval run, the live agent, or the embedded build reads its
/// frame straight out of the current location and carries it forward when it navigates. So one
/// implementation serves all three frames, fully typed, with no per-frame link builders and no `base`
/// string threaded down: the frame travels *in the value*.
export interface Stream {
  frame: StreamFrame;
  view: ViewId;
  /// The open memory (State), room (Conversation), or settings tab (Settings), or `null` at the view's
  /// default.
  selection: string | null;
  /// The pinned cursor, or `null` when following the head.
  seq: number | null;
  search: StreamSearch;
  /// Typed destinations within this frame, for a `<Link to={…}>`.
  link: StreamLink;
  /// Switch views (a tab move): keep only the cursor, dropping the selection and the leaving view's
  /// own search. Pushes, so back and forward step between views.
  selectView: (view: ViewId) => void;
  /// Move the cursor: replace, so scrubbing does not bury the back button; the view and selection stay.
  setSeq: (seq: number | null) => void;
  /// Transform this view's own search keys (a filter toggle, clearing the focus pin): replace.
  patchSearch: (update: (search: StreamSearch) => StreamSearch) => void;
}

/// Typed navigation targets within a fixed frame. Each returns a full [`AppLocation`] carrying that
/// frame, so a shared view builds links without naming — or even knowing — which frame it sits in.
export interface StreamLink {
  view: (view: ViewId, opts?: { seq?: number | null }) => AppLocation;
  state: (memory: string, opts?: { seq?: number | null }) => AppLocation;
  conversation: (opts?: { room?: string; turn?: string; seq?: number | null }) => AppLocation;
  events: (opts?: { focus?: string; seq?: number | null }) => AppLocation;
  settings: (section?: string) => AppLocation;
}

export function streamLink(frame: StreamFrame): StreamLink {
  const at = (stream: StreamView): AppLocation => streamLocation(frame, stream);
  return {
    view: (view, opts) => at({ view, search: seqSearch(opts?.seq) }),
    state: (memory, opts) => at({ view: "state", selection: memory, search: seqSearch(opts?.seq) }),
    conversation: ({ room, turn, seq } = {}) =>
      at({
        view: "conversation",
        ...(room !== undefined ? { selection: room } : {}),
        search: { ...(turn ? { turn } : {}), ...seqSearch(seq) },
      }),
    events: ({ focus, seq } = {}) =>
      at({ view: "events", search: { ...(focus ? { focus } : {}), ...seqSearch(seq) } }),
    settings: (section) =>
      at(
        section !== undefined
          ? { view: "settings", selection: section, search: {} }
          : { view: "settings", search: {} },
      ),
  };
}

/// The stream window for a component that is always rendered inside a stream frame; throws otherwise.
export function useStream(): Stream {
  const stream = useOptionalStream();
  if (!stream) throw new Error("useStream must be used within a stream frame");
  return stream;
}

/// The stream window, or `null` when the current location is not a stream frame — for a component that
/// may also render frameless (an event detail rendered outside a stream), where it degrades to plain,
/// unlinked text rather than crashing.
export function useOptionalStream(): Stream | null {
  const location = useLocation();
  const navigate = useNavigate();
  return buildStream(location, navigate);
}

function buildStream(location: AppLocation, navigate: Navigate): Stream | null {
  const parts = streamPartsOf(location);
  if (!parts) return null;
  const { frame, stream } = parts;
  const link = streamLink(frame);
  const seq = stream.search.seq ?? null;
  return {
    frame,
    view: stream.view,
    selection: stream.selection ?? null,
    seq,
    search: stream.search,
    link,
    selectView: (view) => navigate(link.view(view, { seq })),
    setSeq: (next) =>
      navigate(streamLocation(frame, { ...stream, search: withSeq(stream.search, next) }), {
        replace: true,
      }),
    patchSearch: (update) =>
      navigate(streamLocation(frame, { ...stream, search: update(stream.search) }), {
        replace: true,
      }),
  };
}

function seqSearch(seq: number | null | undefined): StreamSearch {
  return seq != null ? { seq } : {};
}

function withSeq(search: StreamSearch, seq: number | null): StreamSearch {
  const next = { ...search };
  if (seq === null) delete next.seq;
  else next.seq = seq;
  return next;
}
