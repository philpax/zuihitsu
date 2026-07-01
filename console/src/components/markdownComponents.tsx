import { type Components } from "react-markdown";

// The element styling for agent-authored Markdown, shared by `TurnMarkdown` and `ThinkingMarkdown`.
// These are styling config (`Components` maps), not React components in their own right, so they live
// apart from the two component files — which then each export a single component, the shape Fast
// Refresh wants. react-markdown ignores raw HTML in the source, so no untrusted markup reaches the DOM.

/// Element styling for an agent conversation turn rendered as Markdown, in the console's tokens (the
/// body type scale, clay links, and the Lua-block aesthetic — `bg-oat/50`, monospace — for code).
/// Defined once at the module level so the React Compiler sees a stable object rather than a fresh one
/// each render.
export const turnComponents: Components = {
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
    <pre className="mt-3 overflow-auto whitespace-pre-wrap bg-oat/50 px-3 py-2 font-mono text-xs leading-relaxed">
      {children}
    </pre>
  ),
  // GFM tables: wrapped for horizontal scroll (the transcript column is narrow), with a header
  // row set apart by a border and weight. Cell alignment honors the colon-based markers (`:---`,
  // `---:`, `:---:`) via the `style` prop react-markdown passes through.
  table: ({ children }) => (
    <div className="mt-3 overflow-x-auto">
      <table className="w-full border-collapse text-sm">{children}</table>
    </div>
  ),
  thead: ({ children }) => <thead className="border-b border-line">{children}</thead>,
  th: ({ children, style }) => (
    <th className="px-2 py-1 text-left font-semibold text-ink" style={style}>
      {children}
    </th>
  ),
  td: ({ children, style }) => (
    <td className="border-b border-line/50 px-2 py-1 text-ink" style={style}>
      {children}
    </td>
  ),
};

/// The thinking register for the agent's deliberation (reasoning) blocks: the same Markdown structure
/// as a turn, but rendered quiet, smaller, and italic so thinking reads apart from the agent's actual
/// replies. Only the prose elements are muted; code stays upright and monospace (italic monospace
/// reads badly), and the rest fall back to the turn styling.
export const thinkingComponents: Components = {
  ...turnComponents,
  p: ({ children }) => (
    <p className="text-sm italic leading-relaxed text-ink-soft [&:not(:first-child)]:mt-2">
      {children}
    </p>
  ),
  ul: ({ children }) => (
    <ul className="mt-1.5 list-disc space-y-1 pl-5 text-sm italic leading-relaxed text-ink-soft">
      {children}
    </ul>
  ),
  ol: ({ children }) => (
    <ol className="mt-1.5 list-decimal space-y-1 pl-5 text-sm italic leading-relaxed text-ink-soft">
      {children}
    </ol>
  ),
  code: ({ className, children }) => {
    const isBlock = (className ?? "").includes("language-") || String(children).includes("\n");
    return isBlock ? (
      <code className="font-mono not-italic">{children}</code>
    ) : (
      <code className="rounded bg-oat/50 px-1 py-0.5 font-mono text-[0.9em] not-italic">
        {children}
      </code>
    );
  },
  pre: ({ children }) => (
    <pre className="mt-2 overflow-auto whitespace-pre-wrap bg-oat/50 px-3 py-2 font-mono text-xs not-italic leading-relaxed text-ink-soft">
      {children}
    </pre>
  ),
  td: ({ children, style }) => (
    <td
      className="border-b border-line/50 px-2 py-1 text-sm not-italic text-ink-soft"
      style={style}
    >
      {children}
    </td>
  ),
};
