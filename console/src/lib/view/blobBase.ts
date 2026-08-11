import { createContext, useContext } from "react";

/// The origin an attachment's bytes are reachable at, or `null` when nothing serves them. The console
/// is a read replica that folds the log locally and asks no server for state, but bytes are not state:
/// a turn's attachments record only a content address, and the bytes behind it live on the agent that
/// stored them.
///
/// So the fact a view needs is whether *this* console has an agent behind it. The live frame provides
/// its connection's base URL; the eval frame provides nothing and inherits `null`, because an eval
/// package is a finished log with no server behind it and a blob URL there would resolve to nothing.
/// A view keys its rendering off this: with a base an image renders and text can be excerpted, and
/// without one every attachment degrades to a name, type, and size it can state from the record alone.
///
/// A context rather than a prop because the fact is the frame's and only the leaves act on it — the
/// workspace and the transcript between them have nothing to say about it. The empty string is a
/// meaningful value (a same-origin console), so absence is `null`, never falsiness.
export const BlobBase = createContext<string | null>(null);

export function useBlobBase(): string | null {
  return useContext(BlobBase);
}
