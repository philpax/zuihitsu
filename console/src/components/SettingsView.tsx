import { useEffect, useState } from "react";

import type { LiveConnection } from "../lib/live.ts";
import { type Settings, getSettings, putSettings } from "../lib/settings.ts";
import { Eyebrow } from "./primitives.tsx";

/// One leaf field's value, and a record of them — the structural shape the generic editor walks. The
/// public API stays typed against the exported `Settings`; this is only the editor's view of it.
type FieldValue = number | string | boolean;
type FieldRecord = { [key: string]: FieldValue | FieldRecord };

/// The Settings view: the agent's behavioral settings (the latest `ConfigSet` snapshot), read and
/// edited live (spec §Initialization → configuration). A save logs a new operator `ConfigSet` that
/// takes effect on the next read, so the change shows up in the Events view and time-travels like
/// anything else. Distinct from the environmental TOML config, which is read at boot and not editable
/// here.
export function SettingsView({ connection }: { connection: LiveConnection }) {
  const [tree, setTree] = useState<Settings | null>(null);
  const [original, setOriginal] = useState<string>("");
  const [status, setStatus] = useState<"loading" | "ready" | "saving" | "error">("loading");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    getSettings(connection).then(
      (settings) => {
        if (cancelled) return;
        setTree(settings);
        setOriginal(JSON.stringify(settings));
        setStatus("ready");
      },
      (cause) => {
        if (cancelled) return;
        setError(cause instanceof Error ? cause.message : String(cause));
        setStatus("error");
      },
    );
    return () => {
      cancelled = true;
    };
  }, [connection]);

  function update(path: string[], value: FieldValue) {
    // The editor walks the settings structurally; cast at this seam, the typed `Settings` stays the
    // public contract on either side (the fetch and the save).
    setTree((prev) =>
      prev ? (setIn(prev as unknown as FieldRecord, path, value) as unknown as Settings) : prev,
    );
  }

  async function save() {
    if (!tree) return;
    setStatus("saving");
    setError(null);
    try {
      await putSettings(connection, tree);
      setOriginal(JSON.stringify(tree));
      setStatus("ready");
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
      setStatus("error");
    }
  }

  if (status === "loading" || !tree) {
    return (
      <p className="py-16 text-center text-sm text-ink-faint">
        {status === "error" ? `Could not load settings — ${error}` : "Loading settings…"}
      </p>
    );
  }

  const dirty = JSON.stringify(tree) !== original;
  return (
    <div className="mx-auto max-w-prose">
      <header className="mb-8">
        <h2 className="font-serif text-xl text-ink sm:text-2xl">Settings</h2>
        <p className="mt-1 max-w-prose text-sm leading-relaxed text-ink-soft">
          The agent's behavioral settings. A save logs an operator <code>ConfigSet</code> and takes
          effect on the next read.
        </p>
      </header>

      <div className="flex flex-col gap-8">
        {Object.entries(tree as unknown as FieldRecord).map(([section, value]) => (
          <section key={section}>
            <Eyebrow>{label(section)}</Eyebrow>
            <div className="mt-3">
              <Fields tree={value} path={[section]} onChange={update} />
            </div>
          </section>
        ))}
      </div>

      <div className="mt-8 flex items-center gap-4 border-t border-line pt-5">
        <button
          onClick={save}
          disabled={!dirty || status === "saving"}
          className="border border-line-strong px-5 py-2 text-base text-ink transition-colors enabled:hover:border-clay enabled:hover:text-clay disabled:opacity-45"
        >
          {status === "saving" ? "Saving…" : "Save"}
        </button>
        {status === "error" && <span className="font-mono text-2xs text-clay">{error}</span>}
        {!dirty && status === "ready" && (
          <span className="font-mono text-2xs text-ink-faint">no unsaved changes</span>
        )}
      </div>
    </div>
  );
}

/// Render a value tree's fields: a scalar as a labeled input, a nested section indented under its
/// name (so `search.recency.tau_days` reads as a tree).
function Fields({
  tree,
  path,
  onChange,
}: {
  tree: FieldValue | FieldRecord;
  path: string[];
  onChange: (path: string[], value: FieldValue) => void;
}) {
  if (typeof tree !== "object") return null;
  return (
    <div className="flex flex-col gap-3">
      {Object.entries(tree).map(([key, value]) => {
        const here = [...path, key];
        if (typeof value === "object") {
          return (
            <div key={key} className="border-l border-line pl-4">
              <Eyebrow>{label(key)}</Eyebrow>
              <div className="mt-2">
                <Fields tree={value} path={here} onChange={onChange} />
              </div>
            </div>
          );
        }
        return (
          <Leaf key={key} name={key} value={value} onChange={(next) => onChange(here, next)} />
        );
      })}
    </div>
  );
}

const CAPTURE_LEVELS = ["Full", "Digest", "Off"];

function Leaf({
  name,
  value,
  onChange,
}: {
  name: string;
  value: number | string | boolean;
  onChange: (value: FieldValue) => void;
}) {
  const input =
    name === "capture_model_calls" ? (
      <select
        value={String(value)}
        onChange={(event) => onChange(event.target.value)}
        className="border-b border-line bg-transparent pb-1 font-mono text-xs text-ink focus:border-ink-faint focus:outline-none"
      >
        {CAPTURE_LEVELS.map((option) => (
          <option key={option} value={option}>
            {option}
          </option>
        ))}
      </select>
    ) : typeof value === "boolean" ? (
      <input
        type="checkbox"
        checked={value}
        onChange={(event) => onChange(event.target.checked)}
        className="accent-clay"
      />
    ) : (
      <input
        type={typeof value === "number" ? "number" : "text"}
        value={String(value)}
        onChange={(event) =>
          onChange(typeof value === "number" ? Number(event.target.value) : event.target.value)
        }
        className="w-32 border-b border-line bg-transparent pb-1 text-right font-mono text-xs text-ink focus:border-ink-faint focus:outline-none"
      />
    );
  return (
    <label className="flex items-baseline justify-between gap-4">
      <span className="font-mono text-2xs text-ink-soft">{label(name)}</span>
      {input}
    </label>
  );
}

/// A snake_case key as words — `token_budget` → "token budget".
function label(key: string): string {
  return key.replace(/_/g, " ");
}

/// Immutably set a nested value at `path`.
function setIn(tree: FieldRecord, path: string[], value: FieldValue): FieldRecord {
  const [head, ...rest] = path;
  return {
    ...tree,
    [head]: rest.length === 0 ? value : setIn(tree[head] as FieldRecord, rest, value),
  };
}
