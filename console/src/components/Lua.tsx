import hljs from "highlight.js/lib/core";
import lua from "highlight.js/lib/languages/lua";

hljs.registerLanguage("lua", lua);

/// A Lua code block, highlighted by highlight.js and themed to the Japandi palette in app.css (clay
/// keywords, sage strings, faint comments) rather than any stock editor theme.
export function Lua({ code }: { code: string }) {
  const html = hljs.highlight(code, { language: "lua" }).value;
  return (
    <pre className="overflow-auto whitespace-pre-wrap bg-oat/50 px-3 py-2 font-mono text-2xs leading-relaxed">
      <code className="hljs" dangerouslySetInnerHTML={{ __html: html }} />
    </pre>
  );
}
