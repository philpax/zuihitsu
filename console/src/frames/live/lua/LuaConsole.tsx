import { useEffect, useState } from "react";

import type { LiveConnection } from "../../../lib/api/live.ts";
import { type ApiEntry, type LuaOutcome, luaApi, runLua } from "../../../lib/api/lua.ts";
import {
  Button,
  Checkbox,
  Disclosure,
  Eyebrow,
  Hint,
  TextInput,
} from "../../../components/primitives.tsx";
import { CodeEditor } from "./CodeEditor.tsx";
import { ApiReference } from "./ApiReference.tsx";
import { Lua } from "../../../components/Lua.tsx";

/// Read-only one-liners offered while the scrollback is empty — click one to load it into the
/// editor. Each is a safe read against the live graph, chosen to show the console's range: recall,
/// a memory's contents, and its full record.
const STARTERS = [
  'return memory.search("what do I know about…")',
  'return memory.get("self"):entries()',
  'return memory.get("self"):history()',
];

/// One run kept in the console's scrollback: the script and what it returned (a rendered value or an
/// error/abort), or `pending` while in flight.
interface Run {
  id: number;
  script: string;
  outcome: LuaOutcome | null;
  failure: string | null;
}

/// The operator Lua console: an editor that runs ad-hoc Lua against the agent's live graph in a
/// no-commit sandbox (spec §Observability → the operator Lua console). Reads see real memory; nothing
/// the block writes persists, so it is safe to poke at anything — `memory.search("…")`, `mem:history`,
/// the calendar. MCP is off unless opted in (a real external effect, even in the sandbox). The agent's
/// own Lua API stands alongside as a filterable reference — the editor and the reference are the two
/// halves of the same act, writing a call and knowing what to call.
export function LuaConsole({ connection }: { connection: LiveConnection }) {
  const [script, setScript] = useState(
    '-- read-only: nothing here persists\nreturn memory.get("self"):entries()',
  );
  const [allowMcp, setAllowMcp] = useState(false);
  const [allowWeb, setAllowWeb] = useState(false);
  const [pending, setPending] = useState(false);
  const [runs, setRuns] = useState<Run[]>([]);
  const [api, setApi] = useState<ApiEntry[] | null>(null);
  // The API reference is always open beside the editor on a wide screen; on a narrow one it folds
  // behind a disclosure so the editor and scrollback keep the screen.
  const [showApiOnMobile, setShowApiOnMobile] = useState(false);

  useEffect(() => {
    let cancelled = false;
    luaApi(connection).then(
      (entries) => !cancelled && setApi(entries),
      () => !cancelled && setApi([]),
    );
    return () => {
      cancelled = true;
    };
  }, [connection]);

  async function run() {
    const text = script.trim();
    if (!text || pending) return;
    setPending(true);
    const id = runs.length;
    try {
      const outcome = await runLua(connection, text, allowMcp, allowWeb);
      setRuns((prev) => [{ id, script: text, outcome, failure: null }, ...prev]);
    } catch (cause) {
      const failure = cause instanceof Error ? cause.message : String(cause);
      setRuns((prev) => [{ id, script: text, outcome: null, failure }, ...prev]);
    } finally {
      setPending(false);
    }
  }

  return (
    // On a wide screen the editor column is capped and the whole spread centers, so the editor and
    // the reference read as one composed page rather than two islands pinned to opposite edges. The
    // API panel is mounted per breakpoint — folded between the editor and the scrollback on a
    // narrow screen, an always-open sticky column on a wide one — so neither placement distorts
    // the other's flow.
    <div className="lg:grid lg:grid-cols-[minmax(0,46rem)_minmax(0,24rem)] lg:items-start lg:justify-center lg:gap-x-12">
      <div className="min-w-0">
        <CodeEditor value={script} onChange={setScript} onSubmit={run} disabled={pending} />

        <div className="mt-3 flex flex-wrap items-center justify-between gap-x-5 gap-y-2">
          <div className="flex items-center gap-4">
            <Button primary onClick={run} disabled={pending || script.trim().length === 0}>
              {pending ? "running…" : "run"}
            </Button>
            <Checkbox
              checked={allowMcp}
              onChange={setAllowMcp}
              label={
                <>
                  allow MCP
                  <span
                    className="text-ink-faint/60"
                    title="An MCP call reaches external servers for real, even in the sandbox."
                  >
                    (real I/O)
                  </span>
                </>
              }
            />
            <Checkbox
              checked={allowWeb}
              onChange={setAllowWeb}
              label={
                <>
                  allow web
                  <span
                    className="text-ink-faint/60"
                    title="web.markdown fetches a real page over the network, even in the sandbox."
                  >
                    (real I/O)
                  </span>
                </>
              }
            />
          </div>
          <Hint className="hidden sm:inline">⌘/ctrl + ↵ to run</Hint>
        </div>

        <div className="mt-6 lg:hidden">
          <ApiPanel
            api={api}
            allowMcp={allowMcp}
            allowWeb={allowWeb}
            open={showApiOnMobile}
            onToggle={() => setShowApiOnMobile(!showApiOnMobile)}
          />
        </div>

        <section className="mt-6 min-w-0">
          {runs.length === 0 && (
            <div className="border-t border-line pt-4">
              <Eyebrow>try</Eyebrow>
              <ul className="mt-2.5 flex flex-col gap-1.5">
                {STARTERS.map((starter) => (
                  <li key={starter}>
                    <button
                      onClick={() => setScript(starter)}
                      className="text-left font-mono text-xs text-ink-soft transition-colors hover:text-clay"
                      title="Load into the editor"
                    >
                      {starter}
                    </button>
                  </li>
                ))}
              </ul>
            </div>
          )}
          {runs.length > 0 && (
            <div className="mb-4 flex items-baseline justify-between border-b border-line pb-2">
              <Eyebrow>scrollback</Eyebrow>
              <button
                onClick={() => setRuns([])}
                className="font-mono text-xs text-ink-faint transition-colors hover:text-clay"
              >
                clear
              </button>
            </div>
          )}
          <ol className="flex flex-col gap-6">
            {runs.map((entry) => (
              <RunResult key={entry.id} run={entry} />
            ))}
          </ol>
        </section>
      </div>

      <aside className="hidden min-w-0 lg:sticky lg:top-4 lg:block lg:self-start">
        <ApiPanel api={api} allowMcp={allowMcp} allowWeb={allowWeb} open onToggle={() => {}} />
      </aside>
    </div>
  );
}

/// The Lua API panel: a name/doc filter over the same catalogue the agent's system prompt is built
/// from. Always open on a wide screen — the reference is half the point of the console — and folded
/// behind a disclosure on a narrow one.
function ApiPanel({
  api,
  allowMcp,
  allowWeb,
  open,
  onToggle,
}: {
  api: ApiEntry[] | null;
  allowMcp: boolean;
  allowWeb: boolean;
  open: boolean;
  onToggle: () => void;
}) {
  const [query, setQuery] = useState("");
  const needle = query.trim().toLowerCase();
  const filtered =
    api === null
      ? null
      : needle === ""
        ? api
        : api.filter((entry) => `${entry.call} ${entry.doc}`.toLowerCase().includes(needle));

  const summary = api === null ? "loading…" : `${api.length} calls`;
  return (
    <>
      <Disclosure
        open={open}
        onToggle={onToggle}
        label="Lua API"
        summary={summary}
        className="lg:hidden"
      />
      <div className={(open ? "mt-4 block" : "hidden") + " lg:mt-0 lg:block"}>
        <div className="hidden items-baseline justify-between lg:flex">
          <Eyebrow>Lua API</Eyebrow>
          <Hint className="text-2xs">{summary}</Hint>
        </div>
        <TextInput
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder="filter — a name or a word from the doc"
          aria-label="Filter the Lua API"
          className="mt-3"
        />
        {filtered && (
          <div className="mt-5 overflow-y-auto pr-2 lg:max-h-[calc(100vh-12rem)]">
            {filtered.length === 0 ? (
              <Hint>nothing matches “{query.trim()}”</Hint>
            ) : (
              <ApiReference entries={filtered} allowMcp={allowMcp} allowWeb={allowWeb} />
            )}
          </div>
        )}
      </div>
    </>
  );
}

function RunResult({ run }: { run: Run }) {
  const error = run.failure ?? run.outcome?.error ?? null;
  const result = run.outcome?.result ?? null;
  return (
    <li className="border-l-2 border-line pl-4">
      <Lua code={run.script} />
      {error ? (
        <pre className="mt-2 font-mono text-xs whitespace-pre-wrap text-clay">{error}</pre>
      ) : result !== null && result !== "" ? (
        <pre className="mt-2 font-mono text-xs whitespace-pre-wrap text-ink">{result}</pre>
      ) : (
        <p className="mt-2 font-mono text-xs text-ink-faint italic">nil</p>
      )}
    </li>
  );
}
