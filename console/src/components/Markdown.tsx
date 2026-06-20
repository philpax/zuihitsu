import ReactMarkdown, { type Components } from "react-markdown";

/// Element styling for an agent message rendered as Markdown, in the console's tokens (the body type
/// scale, clay links, and the Lua-block aesthetic — `bg-oat/50`, monospace — for code). Defined once
/// at the module level so the React Compiler sees a stable object rather than a fresh one each render.
/// react-markdown ignores raw HTML in the source, so no untrusted markup reaches the DOM.
const components: Components = {
  p: ({ children }) => (
    <p className="text-base leading-relaxed text-ink [&:not(:first-child)]:mt-3">{children}</p>
  ),
  a: ({ children, href }) => (
    <a
      href={href}
      target="_blank"
      rel="noreferrer"
      className="text-clay underline underline-offset-2 transition-colors hover:text-ink"
    >
      {children}
    </a>
  ),
  ul: ({ children }) => (
    <ul className="mt-2 list-disc space-y-1 pl-5 text-base leading-relaxed text-ink">{children}</ul>
  ),
  ol: ({ children }) => (
    <ol className="mt-2 list-decimal space-y-1 pl-5 text-base leading-relaxed text-ink">
      {children}
    </ol>
  ),
  li: ({ children }) => <li className="pl-1">{children}</li>,
  strong: ({ children }) => <strong className="font-semibold">{children}</strong>,
  em: ({ children }) => <em className="italic">{children}</em>,
  blockquote: ({ children }) => (
    <blockquote className="mt-3 border-l-2 border-line pl-3 text-ink-soft">{children}</blockquote>
  ),
  // Headings demoted (a message is nested in the page) and styled down the scale: serif for the top
  // three, the mono eyebrow for the deeper ones, so no level falls back to the unstyled default.
  h1: ({ children }) => <h3 className="mt-4 font-serif text-lg text-ink">{children}</h3>,
  h2: ({ children }) => <h4 className="mt-3 font-serif text-base text-ink">{children}</h4>,
  h3: ({ children }) => (
    <h5 className="mt-3 font-serif text-sm font-medium text-ink">{children}</h5>
  ),
  h4: ({ children }) => (
    <h6 className="mt-3 font-mono text-2xs uppercase tracking-widest text-ink-soft">{children}</h6>
  ),
  h5: ({ children }) => (
    <h6 className="mt-3 font-mono text-2xs uppercase tracking-widest text-ink-faint">{children}</h6>
  ),
  h6: ({ children }) => (
    <h6 className="mt-3 font-mono text-2xs uppercase tracking-widest text-ink-faint">{children}</h6>
  ),
  hr: () => <hr className="my-4 border-line" />,
  // A fenced block (multi-line, or carrying a language class) is laid out by `pre`; only inline code
  // gets its own chip, so block code is not double-boxed.
  code: ({ className, children }) => {
    const isBlock = (className ?? "").includes("language-") || String(children).includes("\n");
    return isBlock ? (
      <code className="font-mono">{children}</code>
    ) : (
      <code className="rounded bg-oat/50 px-1 py-0.5 font-mono text-[0.9em]">{children}</code>
    );
  },
  pre: ({ children }) => (
    <pre className="mt-3 overflow-auto whitespace-pre-wrap bg-oat/50 px-3 py-2 font-mono text-2xs leading-relaxed">
      {children}
    </pre>
  ),
};

/// An agent message rendered as Markdown — paragraphs, emphasis, lists, links, and fenced code blocks
/// — in the console's tokens. The operator's and participants' own input stays raw text (only its
/// newlines are preserved); just the agent's prose, which it composes as Markdown, is rendered here.
export function Markdown({ text }: { text: string }) {
  return <ReactMarkdown components={components}>{text}</ReactMarkdown>;
}
