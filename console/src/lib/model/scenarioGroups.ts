import type { Category } from "../../types/Category.ts";
import type { ScenarioReport } from "../../types/ScenarioReport.ts";

/// The category display order — the Rust enum's own order (`crates/eval/src/package.rs`), which
/// groups the behavioural families semantically rather than alphabetically. A category missing from
/// this list (the set grows over time) sorts after the known ones, in first-seen order, so a new
/// family degrades to "at the end" rather than vanishing.
const CATEGORY_ORDER: readonly Category[] = [
  "recall",
  "tagging",
  "relations",
  "scheduling",
  "privacy",
  "compaction",
  "arbitration",
  "description",
];

/// A category's scenarios, each carrying its index in `pkg.scenarios` — the coordinate the live-run
/// maps (`activeScenarios`, `liveRunOf`) are keyed by, which grouping must not disturb.
export type ScenarioGroup = {
  category: Category;
  entries: { scenario: ScenarioReport; index: number }[];
};

/// Group a package's scenarios by category, in the enum's semantic order, keeping each group's
/// scenarios in their package order. Categories with no scenarios in the package produce no group.
export function groupScenariosByCategory(scenarios: ScenarioReport[]): ScenarioGroup[] {
  const groups = new Map<Category, ScenarioGroup>();
  for (const category of CATEGORY_ORDER) {
    groups.set(category, { category, entries: [] });
  }
  scenarios.forEach((scenario, index) => {
    const category = scenario.meta.category;
    let group = groups.get(category);
    if (!group) {
      group = { category, entries: [] };
      groups.set(category, group);
    }
    group.entries.push({ scenario, index });
  });
  return [...groups.values()].filter((group) => group.entries.length > 0);
}
