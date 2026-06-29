import { useEffect, useState } from "react";

import type { LiveConnection } from "../lib/live.ts";
import { type Settings, getSettings, putSettings } from "../lib/settings.ts";
import { type ConfigTree, type ConfigValue, getConfig } from "../lib/config.ts";
import { snapshotNow } from "../lib/operator.ts";
import { settingsMetadata } from "../types/settings-metadata.ts";
import { Checkbox, Eyebrow } from "./primitives.tsx";

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
  const [config, setConfig] = useState<ConfigTree | null>(null);
  const [snapshot, setSnapshot] = useState<
    | { state: "idle" | "working" }
    | { state: "done"; message: string }
    | { state: "error"; message: string }
  >({ state: "idle" });

  useEffect(() => {
    let cancelled = false;
    // The environmental config is read-only and non-essential, so a failure just hides its section.
    getConfig(connection).then(
      (value) => !cancelled && setConfig(value),
      () => {},
    );
    return () => {
      cancelled = true;
    };
  }, [connection]);

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

  async function takeSnapshot() {
    setSnapshot({ state: "working" });
    try {
      const written = await snapshotNow(connection);
      setSnapshot({
        state: "done",
        message: written ? `Wrote ${written}` : "Already at head — nothing new to snapshot.",
      });
    } catch (cause) {
      setSnapshot({
        state: "error",
        message: cause instanceof Error ? cause.message : String(cause),
      });
    }
  }

  if (status === "loading" || !tree) {
    return (
      <p className="py-12 text-center text-sm text-ink-faint">
        {status === "error" ? `Could not load settings — ${error}` : "Loading settings…"}
      </p>
    );
  }

  const dirty = JSON.stringify(tree) !== original;
  return (
    <div>
      <p className="mb-6 max-w-prose text-sm leading-relaxed text-ink-soft">
        The agent's behavioral settings. A save logs an operator <code>ConfigSet</code> and takes
        effect on the next read.
      </p>

      <div className="flex flex-col gap-6">
        {Object.entries(tree as unknown as FieldRecord).map(([section, value]) => (
          <section key={section}>
            <Eyebrow>{label(section)}</Eyebrow>
            <div className="mt-3">
              <Fields tree={value} path={[section]} onChange={update} />
            </div>
          </section>
        ))}
      </div>

      <div className="mt-6 flex items-center gap-4 border-t border-line pt-5">
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

      <section className="mt-8 border-t border-line pt-6">
        <Eyebrow>Maintenance</Eyebrow>
        <p className="mt-3 max-w-prose text-sm leading-relaxed text-ink-soft">
          Write a graph snapshot now — the take-one-before-an-experiment trigger. Boot restores from
          the latest snapshot and replays only the tail, so a fresh one shortens the next startup.
        </p>
        <div className="mt-4 flex items-center gap-4">
          <button
            onClick={takeSnapshot}
            disabled={snapshot.state === "working"}
            className="border border-line-strong px-5 py-2 text-base text-ink transition-colors enabled:hover:border-clay enabled:hover:text-clay disabled:opacity-45"
          >
            {snapshot.state === "working" ? "Snapshotting…" : "Snapshot now"}
          </button>
          {snapshot.state === "done" && (
            <span className="font-mono text-2xs text-ink-soft">{snapshot.message}</span>
          )}
          {snapshot.state === "error" && (
            <span className="font-mono text-2xs text-clay">{snapshot.message}</span>
          )}
        </div>
      </section>

      {config && (
        <section className="mt-8 border-t border-line pt-6">
          <Eyebrow>Environment</Eyebrow>
          <p className="mt-3 max-w-prose text-sm leading-relaxed text-ink-soft">
            The TOML config this instance booted from — read-only here (it is read at startup, not
            from the log). Secrets are redacted: API keys show as counts, MCP env as its variable
            names.
          </p>
          <div className="mt-5 flex flex-col gap-7">
            {Object.entries(config).map(([section, value]) => (
              <div key={section}>
                <Eyebrow>{label(section)}</Eyebrow>
                <div className="mt-3">
                  <ConfigFields value={value} />
                </div>
              </div>
            ))}
          </div>
        </section>
      )}
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
          <Leaf
            key={key}
            name={key}
            path={here}
            value={value}
            onChange={(next) => onChange(here, next)}
          />
        );
      })}
    </div>
  );
}

const CAPTURE_LEVELS = ["Full", "Digest", "Off"];

function Leaf({
  name,
  path,
  value,
  onChange,
}: {
  name: string;
  path: string[];
  value: number | string | boolean;
  onChange: (value: FieldValue) => void;
}) {
  const meta = settingsMetadata[path.join(".")];
  const isMinutes = meta?.display === "min" && meta?.unit === "seconds";
  // The wire value is seconds; the editor shows minutes. Round the display to two decimals so a
  // non-multiple-of-60 (e.g. 100s) shows `1.67` rather than a float soup; `Math.round` on save
  // recovers the nearest whole second.
  const shownValue =
    isMinutes && typeof value === "number"
      ? String(Number((value / 60).toFixed(2)))
      : String(value);

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
      <Checkbox checked={value} onChange={onChange} />
    ) : (
      <input
        type={typeof value === "number" ? "number" : "text"}
        value={shownValue}
        onChange={(event) => {
          if (isMinutes && typeof value === "number") {
            onChange(Math.round(Number(event.target.value) * 60));
          } else {
            onChange(typeof value === "number" ? Number(event.target.value) : event.target.value);
          }
        }}
        className="w-32 border-b border-line bg-transparent pb-1 text-right font-mono text-xs text-ink focus:border-ink-faint focus:outline-none"
      />
    );
  return (
    <label className="flex items-baseline justify-between gap-4">
      <span className="flex flex-col gap-0.5">
        <span className="font-mono text-2xs text-ink-soft">{label(name)}</span>
        {meta?.description && (
          <span className="text-2xs leading-snug text-ink-faint">{meta.description}</span>
        )}
      </span>
      <span className="flex items-baseline gap-1.5">
        {input}
        {meta?.display && (
          <span className="w-12 text-left font-mono text-2xs text-ink-faint">{meta.display}</span>
        )}
      </span>
    </label>
  );
}

/// Render the environmental config read-only: a nested object indents under its name, and a scalar
/// or array shows its value (a redacted key count, an endpoint, a path, a list of names).
function ConfigFields({ value }: { value: ConfigValue }) {
  if (!isNestedObject(value)) return <Scalar value={value} />;
  return (
    <div className="flex flex-col gap-1.5">
      {Object.entries(value).map(([key, child]) =>
        isNestedObject(child) ? (
          <div key={key} className="border-l border-line pl-4">
            <Eyebrow>{label(key)}</Eyebrow>
            <div className="mt-2">
              <ConfigFields value={child} />
            </div>
          </div>
        ) : (
          <div key={key} className="flex items-baseline justify-between gap-4">
            <span className="font-mono text-2xs text-ink-faint">{label(key)}</span>
            <Scalar value={child} />
          </div>
        ),
      )}
    </div>
  );
}

function isNestedObject(value: ConfigValue): value is ConfigTree {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function Scalar({ value }: { value: ConfigValue }) {
  const text = Array.isArray(value)
    ? value.length === 0
      ? "—"
      : value.map(String).join(", ")
    : value === null || value === ""
      ? "—"
      : String(value);
  return (
    <span className="max-w-[65%] truncate text-right font-mono text-xs text-ink-soft" title={text}>
      {text}
    </span>
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
