import { useEffect, useState } from "react";

import type { LiveConnection } from "../lib/live.ts";
import { type ApiEntry, type LuaOutcome, luaApi, runLua } from "../lib/lua.ts";
import { Checkbox, Eyebrow } from "./primitives.tsx";
import { CodeEditor } from "./CodeEditor.tsx";
import { ApiReference } from "./ApiReference.tsx";

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
/// own Lua API rides alongside as a reference.
export function LuaConsole({ connection }: { connection: LiveConnection }) {
  const [script, setScript] = useState(
    '-- read-only: nothing here persists\nreturn memory.get("self"):entries()',
  );
  const [allowMcp, setAllowMcp] = useState(false);
  const [pending, setPending] = useState(false);
  const [runs, setRuns] = useState<Run[]>([]);
  const [api, setApi] = useState<ApiEntry[] | null>(null);
  const [showApi, setShowApi] = useState(false);

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
      const outcome = await runLua(connection, text, allowMcp);
      setRuns((prev) => [{ id, script: text, outcome, failure: null }, ...prev]);
    } catch (cause) {
      const failure = cause instanceof Error ? cause.message : String(cause);
      setRuns((prev) => [{ id, script: text, outcome: null, failure }, ...prev]);
    } finally {
      setPending(false);
    }
  }

  return (
    <div className="grid grid-cols-1 gap-6 lg:grid-cols-[1fr_20rem]">
      <div>
        <CodeEditor value={script} onChange={setScript} onSubmit={run} disabled={pending} />

        <div className="mt-3 flex flex-wrap items-center justify-between gap-x-5 gap-y-2">
          <div className="flex items-center gap-4">
            <button
              onClick={run}
              disabled={pending || script.trim().length === 0}
              className="border border-line-strong px-4 py-1.5 font-mono text-xs text-ink transition-colors enabled:hover:border-clay enabled:hover:text-clay disabled:opacity-45"
            >
              {pending ? "running…" : "run"}
            </button>
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
          </div>
          <span className="font-mono text-2xs text-ink-faint">⌘/ctrl + ↵ to run</span>
        </div>

        <ol className="mt-6 flex flex-col gap-6">
          {runs.map((entry) => (
            <RunResult key={entry.id} run={entry} />
          ))}
        </ol>
      </div>

      <aside className="lg:sticky lg:top-4 lg:self-start">
        <button
          onClick={() => setShowApi(!showApi)}
          className="flex items-baseline gap-2 text-left transition-colors hover:text-ink"
        >
          <Eyebrow>{showApi ? "▾ Lua API" : "▸ Lua API"}</Eyebrow>
          <span className="font-mono text-2xs text-ink-faint">
            {api ? `${api.length} calls` : "loading…"}
          </span>
        </button>
        {showApi && api && (
          <div className="mt-5 max-h-[34rem] overflow-y-auto pr-2 lg:max-h-[70vh]">
            <ApiReference entries={api} />
          </div>
        )}
      </aside>
    </div>
  );
}

function RunResult({ run }: { run: Run }) {
  const error = run.failure ?? run.outcome?.error ?? null;
  const result = run.outcome?.result ?? null;
  return (
    <li className="border-l-2 border-line pl-4">
      <pre className="whitespace-pre-wrap font-mono text-2xs text-ink-faint">{run.script}</pre>
      {error ? (
        <pre className="mt-2 whitespace-pre-wrap font-mono text-xs text-clay">{error}</pre>
      ) : result !== null && result !== "" ? (
        <pre className="mt-2 whitespace-pre-wrap font-mono text-xs text-ink">{result}</pre>
      ) : (
        <p className="mt-2 font-mono text-2xs italic text-ink-faint">nil</p>
      )}
    </li>
  );
}
