import type { ComponentProps, ComponentType } from "react";
import ReactMarkdown, { type Components, defaultUrlTransform } from "react-markdown";
import rehypeKatex from "rehype-katex";
import remarkBreaks from "remark-breaks";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";

import { useContext } from "react";

import { TURN_CHIP_SCHEME, remarkTurnRefs } from "../../lib/view/turnRefs.ts";
import { MEM_CHIP_HANDLE_SIGIL, MEM_CHIP_SCHEME, MemRefs } from "../../lib/view/memRefs.ts";
import { stateHandleFromUrl } from "../../lib/nav/refRoutes.ts";
import { turnComponents } from "../../components/markdownComponents.tsx";
import { TurnRefChip } from "./TurnRefs.tsx";
import { MemRefChip } from "./MemRefs.tsx";

/// A conversation turn rendered as Markdown — paragraphs, emphasis, lists, links, fenced code
/// blocks, GFM tables, and LaTeX math — in the console's tokens, with turn references
/// (reference tokens and pasted deep-link URLs) rendered as inline chips. The agent composes
/// its prose as Markdown deliberately, so its blank-line paragraphing carries the structure. A
/// participant or operator types plain text, so `softBreaks` preserves their single newlines as
/// line breaks (as chat surfaces do) while still linking URLs and rendering any Markdown they write.
export function TurnMarkdown({ text, softBreaks }: { text: string; softBreaks?: boolean }) {
  return (
    <ReactMarkdown
      remarkPlugins={softBreaks ? breaksPlugins : plugins}
      rehypePlugins={rehypePlugins}
      components={components}
      urlTransform={keepRefSchemes}
    >
      {text}
    </ReactMarkdown>
  );
}

// Module-level plugin arrays and component maps, so the React Compiler sees stable objects.
const plugins = [remarkGfm, remarkMath, remarkTurnRefs];
const breaksPlugins = [remarkGfm, remarkMath, remarkTurnRefs, remarkBreaks];
const rehypePlugins = [rehypeKatex];

/// The styled anchor from the shared component map, for the non-reference links the override below
/// falls through to.
const BaseAnchor = turnComponents.a as ComponentType<ComponentProps<"a">>;

/// The turn components plus a reference-aware anchor: a `turn-chip:` or `mem-chip:` link (minted by the
/// remark plugin) renders as the matching chip; an ordinary link is matched against the console's own
/// State-view route, and — only when its handle resolves to a memory — renders as a memory chip;
/// everything else keeps the ordinary styled anchor.
const components: Components = {
  ...turnComponents,
  a: ({ href, children, ...rest }) =>
    href?.startsWith(TURN_CHIP_SCHEME) ? (
      <TurnRefChip id={href.slice(TURN_CHIP_SCHEME.length)} />
    ) : href?.startsWith(MEM_CHIP_SCHEME) ? (
      <MemRefChip payload={href.slice(MEM_CHIP_SCHEME.length)} />
    ) : (
      <RefAwareAnchor href={href} {...rest}>
        {children}
      </RefAwareAnchor>
    ),
};

/// An ordinary link that becomes a memory chip when it is a console State-view deep link whose handle
/// resolves to a memory, and stays an ordinary anchor otherwise. Route matching is the frontend's own
/// concern (`stateHandleFromUrl`); an unresolved handle is left as a plain link, never a chip, so a
/// stale or foreign State link reads as the URL it is.
function RefAwareAnchor({ href, children, ...rest }: ComponentProps<"a">) {
  const resolver = useContext(MemRefs);
  const handle = typeof href === "string" ? stateHandleFromUrl(href) : null;
  if (handle !== null && resolver.byHandle(handle) !== null) {
    return <MemRefChip payload={MEM_CHIP_HANDLE_SIGIL + handle} />;
  }
  return (
    <BaseAnchor href={href} {...rest}>
      {children}
    </BaseAnchor>
  );
}

/// react-markdown's URL sanitizer drops unknown protocols; let the plugin's `turn-chip:` and `mem-chip:`
/// schemes through (they never reach the DOM — the anchor override renders them as chips).
function keepRefSchemes(url: string): string {
  return url.startsWith(TURN_CHIP_SCHEME) || url.startsWith(MEM_CHIP_SCHEME)
    ? url
    : defaultUrlTransform(url);
}
