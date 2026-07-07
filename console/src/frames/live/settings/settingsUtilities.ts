import type { ConfigTree, ConfigValue } from "../../../lib/api/config.ts";

/// One leaf field's value, and a record of them — the structural shape the generic editor walks. The
/// public API stays typed against the exported `Settings`; this is only the editor's view of it.
export type FieldValue = number | string | boolean;
export type FieldRecord = { [key: string]: FieldValue | FieldRecord };

/// A snake_case key as words — `token_budget` → "token budget".
export function label(key: string): string {
  return key.replace(/_/g, " ");
}

/// Immutably set a nested value at `path`.
export function setIn(tree: FieldRecord, path: string[], value: FieldValue): FieldRecord {
  const [head, ...rest] = path;
  return {
    ...tree,
    [head]: rest.length === 0 ? value : setIn(tree[head] as FieldRecord, rest, value),
  };
}

export function isNestedObject(value: ConfigValue): value is ConfigTree {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}
