import { createContext } from "react";

/// The workspace's scrolling content well: the `<main>` region between the fixed nav above and the
/// fixed footer below, which is the element the active view scrolls inside. The document itself no
/// longer scrolls, so the views that manage their own scroll — the Conversation transcript (windowing,
/// follow-the-foot, jump-to-latest) and the Events virtualizer — drive this element rather than the
/// window. `null` until the element mounts, and in hosts that render a view outside the workspace
/// (such as the handover tests), where the scroll-driving effects simply stay dormant.
export const ScrollContainer = createContext<HTMLElement | null>(null);
