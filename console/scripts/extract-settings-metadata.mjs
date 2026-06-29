// Generates console/src/types/settings-metadata.ts from the ts-rs-generated settings substruct
// files in console/src/types/. The output is checked in alongside the other generated bindings —
// do not hand-edit it; rerun `./console/regen.sh` after a settings `///` doc comment or schema
// change (the CI `regen` job verifies it stays current).
//
// The script uses only Node built-ins (fs/path), so it needs no npm install. It parses the two
// field layouts ts-rs emits:
//   1. JSDoc-prefixed fields on their own line: a `/** ... */` block precedes `field: type,`.
//   2. Inline fields without doc comments: several comma-separated on one line
//      (e.g. `TauDays = { high: number, medium: number, low: number, };`).
// Extracting the substring between the first `{` and the last `}` (the `export type X = { ... }`
// body, excluding the struct-level JSDoc that precedes `export type`) and matching
// `(?:/**...*/)?field: type` over that body captures both layouts in one pass.
import { readdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";

const TYPES_DIR = new URL("../src/types/", import.meta.url);
const OUTPUT_PATH = new URL("../src/types/settings-metadata.ts", import.meta.url);

// Maps each settings substruct type to its parent path in the Settings tree. The schema is
// append-only and stable, so the paths are hardcoded.
const PARENT_PATH = {
  CompactionSettings: "compaction",
  BriefSettings: "brief",
  TurnSettings: "turn",
  SchedulerSettings: "scheduler",
  ConcurrencySettings: "concurrency",
  ObservabilitySettings: "observability",
  SearchSettings: "search",
  RecencySettings: "search.recency",
  TauDays: "search.recency.tau_days",
};

// Field names whose unit does not follow the suffix convention. Looked up before the suffix table.
const EXPLICIT_UNIT = {
  max_wakeups_per_session: "wake-ups",
  max_concurrent_streams: "streams",
  capture_model_calls: "",
  bonus: "weight",
  cosine: "weight",
  bm25: "weight",
  tag: "weight",
  high: "days",
  medium: "days",
  low: "days",
};

// Field-name suffix → unit. Order matters: more specific suffixes (e.g. `_char_budget`) must come
// before shorter ones (e.g. `_budget`) so they match first.
const SUFFIX_UNIT = [
  ["_seconds", "seconds"],
  ["_days", "days"],
  ["_char_budget", "chars"],
  ["_budget", "tokens"],
  ["_turns", "turns"],
  ["_steps", "steps"],
  ["_items", "items"],
  ["_facts", "facts"],
  ["_cap", "items"],
  ["_attempts", "attempts"],
];

// The short label the editor renders after the input. Mirrors `unit` except for `_seconds`, where
// the value is shown in minutes, so the suffix is `min` to match the displayed value.
function displayUnit(fieldName, unit) {
  if (fieldName.endsWith("_seconds")) return "min";
  return unit;
}

function unitFor(fieldName) {
  if (fieldName in EXPLICIT_UNIT) return EXPLICIT_UNIT[fieldName];
  for (const [suffix, unit] of SUFFIX_UNIT) {
    if (fieldName.endsWith(suffix)) return unit;
  }
  return "";
}

// Extracts the `export type X = { ... }` body: the substring between the `{` that opens the type
// declaration and the last `}`. Locating the declaration with a regex (rather than the first `{` in
// the file) skips `import type { X }` statements and the struct-level JSDoc that precedes `export
// type`, so neither pollutes the per-field matching.
function typeBody(source) {
  const m = source.match(/export\s+type\s+\w+\s*=\s*\{/);
  if (!m) {
    throw new Error(`could not locate an 'export type X = {' declaration`);
  }
  const start = m.index + m[0].length;
  const last = source.lastIndexOf("}");
  if (last === -1 || last <= start) {
    throw new Error(`could not locate the type body's closing '}'`);
  }
  return source.slice(start, last);
}

// Parses the body into an ordered list of { name, description } pairs. The regex matches an
// optional JSDoc block followed by `field_name: type`, capturing both the JSDoc-prefixed and
// inline layouts. `matchAll` skips non-matching positions (where `exec` in a loop would reset
// lastIndex to 0), so it handles the inline-comma-separated layout correctly.
function parseFields(body) {
  const fields = [];
  const re = /(?:\/\*\*[\s\S]*?\*\/\s*)?(\w+):\s*([^,]+)/g;
  for (const match of body.matchAll(re)) {
    const raw = match[0];
    const name = match[1];
    const jsdoc = raw.match(/\/\*\*([\s\S]*?)\*\//);
    const description = jsdoc ? cleanDoc(jsdoc[1]) : "";
    fields.push({ name, description });
  }
  return fields;
}

// Collapses a JSDoc body to a single line: strips leading `*` markers and surrounding whitespace,
// joins multi-line comments with a space.
function cleanDoc(jsdocBody) {
  return jsdocBody
    .split("\n")
    .map((line) => line.replace(/^\s*\*\s?/, "").trim())
    .filter((line) => line.length > 0)
    .join(" ");
}

async function main() {
  const files = await readdir(TYPES_DIR);
  const settingsFiles = files.filter(
    (name) => name.endsWith(".ts") && name.replace(/\.ts$/, "") in PARENT_PATH,
  );

  const entries = [];
  for (const fileName of settingsFiles) {
    const typeName = fileName.replace(/\.ts$/, "");
    const parent = PARENT_PATH[typeName];
    const source = await readFile(join(TYPES_DIR.pathname, fileName), "utf8");
    const body = typeBody(source);
    for (const { name, description } of parseFields(body)) {
      const unit = unitFor(name);
      entries.push({
        path: `${parent}.${name}`,
        description,
        unit,
        display: displayUnit(name, unit),
      });
    }
  }

  // Stable, human-readable ordering: by the dotted path.
  entries.sort((a, b) => a.path.localeCompare(b.path));

  const lines = [
    "// Generated by console/scripts/extract-settings-metadata.mjs from the ts-rs bindings. Do not edit.",
    "// Rerun ./console/regen.sh after a settings doc comment or schema change.",
    "",
    "export interface SettingsFieldMeta {",
    "  description: string;",
    "  unit: string;",
    "  display: string;",
    "}",
    "",
    "export const settingsMetadata: Record<string, SettingsFieldMeta> = {",
  ];
  for (const e of entries) {
    const desc = JSON.stringify(e.description);
    lines.push(
      `  ${JSON.stringify(e.path)}: { description: ${desc}, unit: ${JSON.stringify(e.unit)}, display: ${JSON.stringify(e.display)} },`,
    );
  }
  lines.push("};");
  lines.push("");

  await writeFile(OUTPUT_PATH, lines.join("\n"), "utf8");
  console.log(`==> wrote ${entries.length} field metadata entries to ${OUTPUT_PATH.pathname}`);
}

main().catch((err) => {
  console.error(`extract-settings-metadata: ${err}`);
  process.exit(1);
});
