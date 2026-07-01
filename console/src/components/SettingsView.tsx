import { useEffect, useState } from "react";
import { useSearchParams } from "react-router-dom";

import type { LiveConnection } from "../lib/live.ts";
import { type Settings, getSettings, putSettings } from "../lib/settings.ts";
import { type ConfigTree, type ConfigValue, getConfig } from "../lib/config.ts";
import { snapshotNow } from "../lib/operator.ts";
import { settingsMetadata } from "../types/settings-metadata.ts";
import { Button, Checkbox, Eyebrow, Hint, Segmented } from "./primitives.tsx";

/// One leaf field's value, and a record of them — the structural shape the generic editor walks. The
/// public API stays typed against the exported `Settings`; this is only the editor's view of it.
type FieldValue = number | string | boolean;
type FieldRecord = { [key: string]: FieldValue | FieldRecord };

/// The view's three concerns, each its own section: the agent's behavioral settings (editable, live),
/// the environmental TOML config it booted from (read-only), and maintenance actions. The open
/// section rides in the URL (`?section`), so it deep-links and survives a view switch.
const SECTIONS = [
  { id: "settings", label: "Settings" },
  { id: "environment", label: "Environment" },
  { id: "maintenance", label: "Maintenance" },
] as const;
type SectionId = (typeof SECTIONS)[number]["id"];

/// The Settings view: the agent's behavioral settings (the latest `ConfigSet` snapshot), read and
/// edited live (spec §Initialization → configuration). A save logs a new operator `ConfigSet` that
/// takes effect on the next read, so the change shows up in the Events view and time-travels like
/// anything else.
export function SettingsView({ connection }: { connection: LiveConnection }) {
  const [searchParams, setSearchParams] = useSearchParams();
  const requested = searchParams.get("section");
  const section: SectionId = SECTIONS.some((entry) => entry.id === requested)
    ? (requested as SectionId)
    : "settings";

  function selectSection(id: string) {
    setSearchParams(
      (prev) => {
        const updated = new URLSearchParams(prev);
        updated.set("section", id);
        return updated;
      },
      { replace: true },
    );
  }

  return (
    <div className="max-w-2xl">
      <Segmented options={SECTIONS} value={section} onChange={selectSection} className="mb-6" />
      {section === "settings" && <BehavioralSettings connection={connection} />}
      {section === "environment" && <EnvironmentSection connection={connection} />}
      {section === "maintenance" && <MaintenanceSection connection={connection} />}
    </div>
  );
}

/// The editable behavioral settings tree, with the save bar footing it.
function BehavioralSettings({ connection }: { connection: LiveConnection }) {
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
      <p className="py-12 text-center text-sm text-ink-faint">
        {status === "error" ? `Could not load settings — ${error}` : "Loading settings…"}
      </p>
    );
  }

  const dirty = JSON.stringify(tree) !== original;
  return (
    <div>
      <div className="flex flex-col gap-8">
        {Object.entries(tree as unknown as FieldRecord).map(([group, value]) => (
          <section key={group}>
            <Eyebrow>{label(group)}</Eyebrow>
            <div className="mt-3">
              <Fields tree={value} path={[group]} onChange={update} />
            </div>
          </section>
        ))}
      </div>

      <div className="sticky bottom-0 mt-8 flex items-center gap-4 border-t border-line bg-paper/95 py-4 backdrop-blur">
        <Button primary onClick={save} disabled={!dirty || status === "saving"}>
          {status === "saving" ? "Saving…" : "Save"}
        </Button>
        {status === "error" && <Hint tone="error">{error}</Hint>}
        {!dirty && status === "ready" && <Hint>no unsaved changes</Hint>}
      </div>
    </div>
  );
}

/// The environmental TOML config, read-only.
function EnvironmentSection({ connection }: { connection: LiveConnection }) {
  const [config, setConfig] = useState<ConfigTree | null | "unavailable">(null);

  useEffect(() => {
    let cancelled = false;
    getConfig(connection).then(
      (value) => !cancelled && setConfig(value),
      () => !cancelled && setConfig("unavailable"),
    );
    return () => {
      cancelled = true;
    };
  }, [connection]);

  if (config === null) {
    return <p className="py-12 text-center text-sm text-ink-faint">Loading the environment…</p>;
  }
  if (config === "unavailable") {
    return (
      <p className="py-12 text-center text-sm text-ink-faint">
        The environment is not available from this host.
      </p>
    );
  }
  return (
    <div>
      <p className="max-w-prose text-sm leading-relaxed text-ink-soft">
        The TOML config this instance booted from — read-only here (it is read at startup, not from
        the log). Secrets are redacted: API keys show as counts, MCP env as its variable names.
      </p>
      <div className="mt-6 flex flex-col gap-7">
        {Object.entries(config).map(([group, value]) => (
          <div key={group}>
            <Eyebrow>{label(group)}</Eyebrow>
            <div className="mt-3">
              <ConfigFields value={value} />
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

/// Maintenance actions against the running instance.
function MaintenanceSection({ connection }: { connection: LiveConnection }) {
  const [snapshot, setSnapshot] = useState<
    | { state: "idle" | "working" }
    | { state: "done"; message: string }
    | { state: "error"; message: string }
  >({ state: "idle" });

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

  return (
    <section>
      <Eyebrow>Graph snapshot</Eyebrow>
      <p className="mt-3 max-w-prose text-sm leading-relaxed text-ink-soft">
        Write a graph snapshot now — the take-one-before-an-experiment trigger. Boot restores from
        the latest snapshot and replays only the tail, so a fresh one shortens the next startup.
      </p>
      <div className="mt-4 flex items-center gap-4">
        <Button onClick={takeSnapshot} disabled={snapshot.state === "working"}>
          {snapshot.state === "working" ? "Snapshotting…" : "Snapshot now"}
        </Button>
        {snapshot.state === "done" && <Hint className="text-ink-soft">{snapshot.message}</Hint>}
        {snapshot.state === "error" && <Hint tone="error">{snapshot.message}</Hint>}
      </div>
    </section>
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

/// The display units a time-based field can be edited in. The wire value stays in the field's own
/// unit (seconds or days); only what the input shows converts. Seconds round to whole on save (the
/// wire fields are integers); days keep two decimals (the tau constants are fractional).
interface DisplayUnit {
  id: string;
  factor: number;
}
const SECOND_UNITS: DisplayUnit[] = [
  { id: "s", factor: 1 },
  { id: "min", factor: 60 },
  { id: "h", factor: 3600 },
  { id: "d", factor: 86400 },
];
const DAY_UNITS: DisplayUnit[] = [
  { id: "d", factor: 1 },
  { id: "wk", factor: 7 },
];

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
  const units =
    typeof value === "number" && meta?.unit === "seconds"
      ? SECOND_UNITS
      : typeof value === "number" && meta?.unit === "days"
        ? DAY_UNITS
        : null;
  // The unit the field is being edited in — the metadata's preferred display to start (`min` for the
  // long intervals), switchable per field.
  const [unitId, setUnitId] = useState(
    meta?.display && units?.some((unit) => unit.id === meta.display) ? meta.display : units?.[0].id,
  );
  const unit = units?.find((entry) => entry.id === unitId) ?? null;

  // Round the display to two decimals so a non-multiple (e.g. 100s shown in minutes) reads `1.67`
  // rather than a float soup; the save conversion below recovers the wire unit.
  const shownValue =
    unit && typeof value === "number"
      ? String(Number((value / unit.factor).toFixed(2)))
      : String(value);

  function onEdit(text: string) {
    if (unit && typeof value === "number") {
      const inWire = Number(text) * unit.factor;
      onChange(meta?.unit === "seconds" ? Math.round(inWire) : Number(inWire.toFixed(2)));
    } else {
      onChange(typeof value === "number" ? Number(text) : text);
    }
  }

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
        onChange={(event) => onEdit(event.target.value)}
        className="w-28 border-b border-line bg-transparent pb-1 text-right font-mono text-xs text-ink focus:border-ink-faint focus:outline-none"
      />
    );
  return (
    <div className="flex items-baseline justify-between gap-4">
      <label className="flex flex-col gap-0.5">
        <span className="font-mono text-2xs text-ink-soft">{label(name)}</span>
        {meta?.description && (
          <span className="max-w-prose text-xs leading-snug text-ink-faint">
            {meta.description}
          </span>
        )}
      </label>
      <span className="flex shrink-0 items-baseline gap-1.5">
        {input}
        {units ? (
          <select
            value={unitId}
            onChange={(event) => setUnitId(event.target.value)}
            aria-label={`Unit for ${label(name)}`}
            className="w-12 bg-transparent font-mono text-2xs text-ink-faint focus:outline-none"
          >
            {units.map((entry) => (
              <option key={entry.id} value={entry.id}>
                {entry.id}
              </option>
            ))}
          </select>
        ) : (
          meta?.display && (
            <span className="w-12 text-left font-mono text-2xs text-ink-faint">{meta.display}</span>
          )
        )}
      </span>
    </div>
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
