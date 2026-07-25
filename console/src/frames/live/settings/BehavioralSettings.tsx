import type { Settings } from "../../../lib/api/settings.ts";
import { type FieldRecord, type FieldValue } from "./settingsUtilities.ts";
import { Fields } from "./fields.tsx";

/// One behavioural settings group (a top-level object of the agent's live `ConfigSet`), rendered as
/// the detail pane of its sidebar section. Purely presentational: the draft, its dirtiness, and the
/// save all live in `SettingsView` so the save bar can stay global to the view (reachable from any
/// section), and this pane only renders the group's fields and reports its own load state.
export function BehavioralSettings({
  tree,
  group,
  status,
  error,
  onChange,
}: {
  tree: Settings | null;
  group: string;
  status: "loading" | "ready" | "saving" | "error";
  error: string | null;
  onChange: (path: string[], value: FieldValue) => void;
}) {
  if (status === "loading" || !tree) {
    return (
      <p className="py-12 text-center text-sm text-ink-faint">
        {status === "error" ? `Could not load settings — ${error}` : "Loading settings…"}
      </p>
    );
  }

  const value = (tree as unknown as FieldRecord)[group];
  if (value === undefined || value === null || typeof value !== "object") {
    return <p className="py-12 text-center text-sm text-ink-faint">No such settings group.</p>;
  }

  // No top-level heading: the sidebar's active row already names the group, so repeating it here
  // would only push the fields down. Sections with internal structure (Environment's TOML groups,
  // Maintenance's sweeps) keep their own sub-headings.
  return (
    <section>
      <Fields tree={value} path={[group]} onChange={onChange} />
    </section>
  );
}
