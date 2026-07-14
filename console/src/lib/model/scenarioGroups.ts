import type { Category } from "@zuihitsu/wire/types/Category.ts";
import type { ScenarioMeta } from "@zuihitsu/wire/types/ScenarioMeta.ts";

/// The category display order — the Rust enum's own order (`crates/eval/src/package.rs`), which
/// groups the behavioural families semantically rather than alphabetically. The `satisfies` clause
/// plus the exhaustiveness check below make membership compiler-checked: a category added to the
/// enum (and regenerated) fails the build here until it is placed, so the order cannot silently
/// drift from the wire type.
const CATEGORY_ORDER = [
  "recall",
  "identity",
  "relations",
  "tagging",
  "time",
  "privacy",
  "sessions",
  "writes",
  "synthesis",
] as const satisfies readonly Category[];

// Every Category must appear in CATEGORY_ORDER: if the generated union gains a member this misses,
// `Missing` is non-never and the assignment fails to typecheck.
type Missing = Exclude<Category, (typeof CATEGORY_ORDER)[number]>;
const _exhaustive: Missing[] = [] satisfies never[];
void _exhaustive;

/// Legacy category names from packages recorded before the module/category alignment, mapped to
/// their absorbing category — mirroring the serde aliases on the Rust enum, because the console
/// parses a package's JSON directly and never round-trips it through serde. An archived package
/// keeps rendering under the current taxonomy rather than falling to the unknown tail.
const LEGACY_CATEGORIES: Record<string, Category> = {
  scheduling: "time",
  compaction: "sessions",
  description: "synthesis",
  arbitration: "synthesis",
};

/// A category's scenarios, each carrying its index in `pkg.scenarios` — the coordinate the live-run
/// maps (`activeScenarios`, `liveRunOf`) are keyed by, which grouping must not disturb. Generic over
/// the scenario shape so it groups both a full [`ScenarioReport`] and a lean [`ScenarioSummary`] —
/// grouping reads only `meta`, which both carry.
export type ScenarioGroup<S extends { meta: ScenarioMeta }> = {
  category: Category;
  entries: { scenario: S; index: number }[];
};

/// Group a package's scenarios by category, in the enum's semantic order, keeping each group's
/// scenarios in their package order. Legacy category names map to their absorbing category; a
/// genuinely unknown one (a future category rendered by an older console) groups at the end in
/// first-seen order rather than vanishing. Categories with no scenarios produce no group.
export function groupScenariosByCategory<S extends { meta: ScenarioMeta }>(
  scenarios: S[],
): ScenarioGroup<S>[] {
  const groups = new Map<Category, ScenarioGroup<S>>();
  for (const category of CATEGORY_ORDER) {
    groups.set(category, { category, entries: [] });
  }
  scenarios.forEach((scenario, index) => {
    const raw = scenario.meta.category as string;
    const category = LEGACY_CATEGORIES[raw] ?? (raw as Category);
    let group = groups.get(category);
    if (!group) {
      group = { category, entries: [] };
      groups.set(category, group);
    }
    group.entries.push({ scenario, index });
  });
  return [...groups.values()].filter((group) => group.entries.length > 0);
}
