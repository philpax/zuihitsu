import type { ComponentProps, ComponentType } from "react";
import ReactMarkdown, { type Components, defaultUrlTransform } from "react-markdown";
import rehypeKatex from "rehype-katex";
import remarkBreaks from "remark-breaks";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";

import { TURNREF_SCHEME, remarkTurnRefs } from "../../lib/view/turnRefs.ts";
import { turnComponents } from "../../components/markdownComponents.tsx";
import { TurnRefChip } from "./TurnRefs.tsx";

/// A conversation turn rendered as Markdown — paragraphs, emphasis, lists, links, fenced code
/// blocks, GFM tables, and LaTeX math — in the console's tokens, with turn references
/// (`[turn:<ulid>]` tokens and pasted deep-link URLs) rendered as inline chips. The agent composes
/// its prose as Markdown deliberately, so its blank-line paragraphing carries the structure. A
/// participant or operator types plain text, so `softBreaks` preserves their single newlines as
/// line breaks (as chat surfaces do) while still linking URLs and rendering any Markdown they write.
export function TurnMarkdown({ text, softBreaks }: { text: string; softBreaks?: boolean }) {
  return (
    <ReactMarkdown
      remarkPlugins={softBreaks ? breaksPlugins : plugins}
      rehypePlugins={rehypePlugins}
      components={components}
      urlTransform={keepTurnRefs}
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

/// The turn components plus a reference-aware anchor: a `turnref:` link (minted by the remark
/// plugin) renders as a chip; everything else keeps the ordinary styled anchor.
const components: Components = {
  ...turnComponents,
  a: ({ href, children, ...rest }) =>
    href?.startsWith(TURNREF_SCHEME) ? (
      <TurnRefChip id={href.slice(TURNREF_SCHEME.length)} />
    ) : (
      <BaseAnchor href={href} {...rest}>
        {children}
      </BaseAnchor>
    ),
};

/// react-markdown's URL sanitizer drops unknown protocols; let the plugin's `turnref:` scheme
/// through (it never reaches the DOM — the anchor override renders it as a chip).
function keepTurnRefs(url: string): string {
  return url.startsWith(TURNREF_SCHEME) ? url : defaultUrlTransform(url);
}
