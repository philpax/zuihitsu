import { useState } from "react";

import { type ConfigValue } from "../../../lib/api/config.ts";
import { settingsMetadata } from "@zuihitsu/wire/types/settings-metadata.ts";
import { Checkbox, Eyebrow } from "../../../components/primitives.tsx";
import { type FieldRecord, type FieldValue, isNestedObject, label } from "./settingsUtilities.ts";

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

/// Render a value tree's fields: a scalar as a labeled input, a nested section indented under its
/// name (so `search.recency.tau_days` reads as a tree).
export function Fields({
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

export function Leaf({
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
export function ConfigFields({ value }: { value: ConfigValue }) {
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

export function Scalar({ value }: { value: ConfigValue }) {
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
