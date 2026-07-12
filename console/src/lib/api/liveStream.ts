import type { Event } from "../../types/Event.ts";
import type { TurnProgress } from "../../types/TurnProgress.ts";

/// A minimal server-sent-events reader over `fetch`, used instead of the native `EventSource`
/// because the control surface authenticates with a bearer header, which `EventSource` cannot
/// carry. Parses the frame types the agent's SSE endpoints emit: committed `event`s, ephemeral
/// `progress` frames, and the platform stream's terminals.
export interface StreamHandlers {
  /// The stream opened (a 200 with a body) — the caller's cue to reset failure counters.
  onOpen?: () => void;
  /// A batch of committed events — consecutive `event` frames from one network chunk, delivered
  /// together so the caller folds once per chunk. Batches never reorder against `progress` frames:
  /// a progress frame flushes the pending batch first, so frames apply in wire order.
  onEvents: (events: Event[]) => void;
  onProgress: (frame: TurnProgress) => void;
  /// The stream ended — an error, or the server closing (which the server does deliberately on
  /// broadcast lag, expecting a reconnect). `error` is `null` on a clean end.
  onClose: (error: Error | null) => void;
}

/// Open the stream and parse frames until it ends or `abort` is called. Returns the abort handle.
export function openEventStream(
  baseUrl: string,
  key: string | null,
  from: number,
  handlers: StreamHandlers,
): () => void {
  const controller = new AbortController();
  (async () => {
    try {
      const headers: HeadersInit = {
        Accept: "text/event-stream",
        ...(key ? { Authorization: `Bearer ${key}` } : {}),
      };
      const response = await fetch(`${baseUrl}/control/events/stream?from=${from}`, {
        headers,
        signal: controller.signal,
      });
      if (!response.ok || !response.body) {
        handlers.onClose(new Error(`the event stream answered ${response.status}`));
        return;
      }
      handlers.onOpen?.();
      const reader = response.body.getReader();
      const decoder = new SseDecoder();
      for (;;) {
        const { done, value } = await reader.read();
        if (done) break;
        // Frames apply in wire order: `event` frames batch until a `progress` frame (or the chunk's
        // end) flushes them, so a progress frame never leapfrogs the commit that supersedes it.
        let batch: Event[] = [];
        const flush = () => {
          if (batch.length > 0) handlers.onEvents(batch);
          batch = [];
        };
        for (const frame of decoder.push(value)) {
          if (frame.kind === "event") batch.push(JSON.parse(frame.data) as Event);
          else if (frame.kind === "progress") {
            flush();
            handlers.onProgress(JSON.parse(frame.data) as TurnProgress);
          }
        }
        flush();
      }
      handlers.onClose(null);
    } catch (cause) {
      // An abort is the caller hanging up, not a failure.
      if (controller.signal.aborted) return;
      handlers.onClose(cause instanceof Error ? cause : new Error(String(cause)));
    }
  })();
  return () => controller.abort();
}

/// One parsed SSE frame: its `event:` name and concatenated `data:` payload.
export interface SseFrame {
  kind: string;
  data: string;
}

/// An incremental SSE decoder: feed it network chunks, get complete frames back, with partial
/// frames buffered across chunk boundaries. Tolerates `\r\n` line endings (the spec allows them;
/// axum emits bare `\n`) and swallows keep-alive/comment frames. Extracted as a pure class so the
/// frame grammar is unit-testable without a socket.
export class SseDecoder {
  private buffered = "";
  private readonly decoder = new TextDecoder();

  push(chunk: Uint8Array): SseFrame[] {
    this.buffered += this.decoder.decode(chunk, { stream: true });
    // Frames are separated by a blank line; anything after the last separator is a partial frame
    // kept for the next chunk.
    const parts = this.buffered.split(/\r?\n\r?\n/);
    this.buffered = parts.pop() ?? "";
    const frames: SseFrame[] = [];
    for (const part of parts) {
      const frame = parseFrame(part);
      if (frame) frames.push(frame);
    }
    return frames;
  }
}

/// One SSE frame's `event:` name and concatenated `data:` payload; `null` for keep-alives and
/// comment frames.
function parseFrame(frame: string): SseFrame | null {
  let kind = "message";
  const data: string[] = [];
  for (const raw of frame.split("\n")) {
    const line = raw.endsWith("\r") ? raw.slice(0, -1) : raw;
    if (line.startsWith("event:")) kind = stripLeadingSpace(line.slice("event:".length));
    else if (line.startsWith("data:")) data.push(stripLeadingSpace(line.slice("data:".length)));
  }
  if (data.length === 0) return null;
  return { kind, data: data.join("\n") };
}

/// The SSE grammar allows exactly one optional space after the field's colon; stripping more (a
/// `trimStart`) would eat whitespace that belongs to the value.
function stripLeadingSpace(value: string): string {
  return value.startsWith(" ") ? value.slice(1) : value;
}
