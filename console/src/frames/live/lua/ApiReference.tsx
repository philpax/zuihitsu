import { type ApiEntry, type ApiGate, formatType } from "../../../lib/api/lua.ts";
import { groupBy } from "../../../lib/format/collections.ts";
import { Eyebrow } from "../../../components/primitives.tsx";

/// The Lua API the agent acts through, projected as a reference — the same structured catalogue
/// `GET /control/lua-api` serves and the agent's own system prompt is built from, so what you read
/// here is exactly what the agent can do. Grouped by namespace, each call showing its parameters and
/// what it returns. A call gated on an opt-in the composer has switched off (`allowMcp`, `allowWeb`)
/// is dimmed and tagged, so the reference never lists a call the current run cannot make.
export function ApiReference({
  entries,
  allowMcp,
  allowWeb,
}: {
  entries: ApiEntry[];
  allowMcp: boolean;
  allowWeb: boolean;
}) {
  const groups = groupByNamespace(entries);
  const isGatedOff = (gate: ApiGate | null) =>
    (gate === "Mcp" && !allowMcp) || (gate === "Web" && !allowWeb);
  return (
    <div className="flex flex-col gap-7">
      {groups.map(([namespace, calls]) => (
        <section key={namespace}>
          <Eyebrow>{namespace}</Eyebrow>
          <ul className="mt-3 flex flex-col gap-4">
            {calls.map((entry) => {
              const gatedOff = isGatedOff(entry.gate);
              return (
                <li key={entry.call} className={gatedOff ? "opacity-55" : undefined}>
                  <p className="font-mono text-xs text-ink">
                    <span className="text-clay">{entry.call}</span>
                    <Signature entry={entry} />
                    {gatedOff && (
                      <span className="ml-2 text-2xs text-clay/80">
                        needs allow {entry.gate === "Web" ? "web" : "MCP"}
                      </span>
                    )}
                  </p>
                  {entry.doc && <p className="mt-1 text-sm/relaxed text-ink-soft">{entry.doc}</p>}
                </li>
              );
            })}
          </ul>
        </section>
      ))}
    </div>
  );
}

function Signature({ entry }: { entry: ApiEntry }) {
  return (
    <span className="text-ink-faint">
      (
      {entry.params.map((param, index) => (
        <span key={param.name}>
          {index > 0 && ", "}
          <span className={param.required ? "text-ink-soft" : "text-ink-faint italic"}>
            {param.name}
          </span>
          <span className="text-ink-faint/70">: {formatType(param.ty)}</span>
        </span>
      ))}
      ) → <span className="text-sage">{formatType(entry.returns)}</span>
    </span>
  );
}

/// Group calls by the namespace before their first `.` or `:` (`memory.create` → `memory`,
/// `mem:append` → `mem`), preserving the catalogue's order within each.
function groupByNamespace(entries: ApiEntry[]): Array<[string, ApiEntry[]]> {
  return groupBy(entries, (entry) => entry.call.match(/^[^.:]+/)?.[0] ?? entry.call);
}
