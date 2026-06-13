import hljs from "highlight.js/lib/core";
import json from "highlight.js/lib/languages/json";

hljs.registerLanguage("json", json);

/// A pretty-printed, Japandi-themed JSON block — the fallback rendering for event payloads without a
/// bespoke view, and for genuinely structured fields like a settings snapshot.
export function Json({ value }: { value: unknown }) {
  const html = hljs.highlight(JSON.stringify(value, null, 2), { language: "json" }).value;
  return (
    <pre className="overflow-auto whitespace-pre-wrap bg-oat/40 px-3 py-2 font-mono text-2xs leading-relaxed">
      <code className="hljs" dangerouslySetInnerHTML={{ __html: html }} />
    </pre>
  );
}
