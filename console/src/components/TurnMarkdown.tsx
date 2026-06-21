import ReactMarkdown from "react-markdown";

import { turnComponents } from "./markdownComponents.tsx";

/// An agent conversation turn rendered as Markdown — paragraphs, emphasis, lists, links, and fenced
/// code blocks — in the console's tokens. The operator's and participants' own input stays raw text
/// (only its newlines are preserved); just the agent's prose, which it composes as Markdown, is here.
export function TurnMarkdown({ text }: { text: string }) {
  return <ReactMarkdown components={turnComponents}>{text}</ReactMarkdown>;
}
