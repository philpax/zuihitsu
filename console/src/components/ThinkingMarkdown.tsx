import ReactMarkdown from "react-markdown";
import rehypeKatex from "rehype-katex";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";

import { thinkingComponents } from "./markdownComponents.tsx";

/// A deliberation (reasoning) block rendered as Markdown, in the quiet, smaller, italic register that
/// sets the agent's thinking apart from its actual turns — same structure, muted tone.
export function ThinkingMarkdown({ text }: { text: string }) {
  return (
    <ReactMarkdown
      remarkPlugins={[remarkGfm, remarkMath]}
      rehypePlugins={[rehypeKatex]}
      components={thinkingComponents}
    >
      {text}
    </ReactMarkdown>
  );
}
