import { useEffect, useState } from "react";

import type { Event } from "@zuihitsu/wire/types/Event.ts";
import type { LiveConnection } from "../../../lib/api/live.ts";
import { Link } from "../../../lib/nav/history.tsx";
import { useNavigate } from "../../../lib/nav/historyContext.ts";
import { useStream } from "../../../lib/nav/useStreamLocation.ts";
import { type Settings, getSettings, putSettings } from "../../../lib/api/settings.ts";
import { Button, Hint, Select } from "../../../components/primitives.tsx";
import { type FieldRecord, type FieldValue, label, setIn } from "./settingsUtilities.ts";
import { FIXED_SECTIONS, type SectionId } from "./sectionConstants.ts";
import { BehavioralSettings } from "./BehavioralSettings.tsx";
import { EnvironmentSection } from "./EnvironmentSection.tsx";
import { MaintenanceSection } from "./MaintenanceSection.tsx";

/// Sentence-case a settings group label, so the derived sections match the fixed tail's casing.
function title(text: string): string {
  return text.charAt(0).toUpperCase() + text.slice(1);
}

/// The Settings view: the agent's behavioral settings (the latest `ConfigSet` snapshot), read and
/// edited live (spec §Initialization → configuration). A save logs a new operator `ConfigSet` that
/// takes effect on the next read, so the change shows up in the Events view and time-travels like
/// anything else.
///
/// Laid out as a master-detail pair, the same motif as the State and Prompts views: the sidebar
/// lists one section per behavioural settings group (compaction, brief, turn, …), then Maintenance
/// and Environment; the selected section renders alone in the detail pane. The maintenance group's
/// fields live inside the Maintenance section, beside the sweep history they govern. The open
/// section rides the URL as the view's selection segment (`/live/settings/<section>`), so it
/// deep-links and browser back and forward walk it; a bare `/…/settings` defaults to the first
/// behavioural group.
///
/// The behavioral draft lives here at the view level, so the save bar stays global: an unsaved edit
/// in any group stays reachable to save no matter which section the operator has since opened, with
/// a hint naming the dirty groups when they are out of sight.
/// On a read-only instance the fields still read and still take edits — the tree is worth composing
/// against — but the save is refused with a `409`, so it is held closed with a note saying why.
export function SettingsView({
  connection,
  events,
  readOnly = false,
}: {
  connection: LiveConnection;
  events: Event[];
  readOnly?: boolean;
}) {
  const navigate = useNavigate();
  const { selection, link } = useStream();

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

  // One sidebar section per behavioural group, in the tree's own (struct) order — the maintenance
  // group folds into the Maintenance section instead of standing alone — then the fixed tail. While
  // the tree loads, only the fixed sections are known.
  const groups = tree
    ? Object.keys(tree as unknown as FieldRecord).filter((group) => group !== "maintenance")
    : [];
  const sections: { id: SectionId; label: string }[] = [
    ...groups.map((id) => ({ id, label: title(label(id)) })),
    ...FIXED_SECTIONS,
  ];
  const section: SectionId = sections.some((entry) => entry.id === selection)
    ? (selection as SectionId)
    : (groups[0] ?? "maintenance");

  // Switching sections is navigation, so it pushes a history entry (back returns to the prior section).
  const selectSection = (id: string) => navigate(link.settings(id));

  function update(path: string[], value: FieldValue) {
    // The editor walks the settings structurally; cast at this seam, the typed `Settings` stays the
    // public contract on either side (the fetch and the save).
    setTree((prev) =>
      prev ? (setIn(prev as unknown as FieldRecord, path, value) as unknown as Settings) : prev,
    );
  }

  async function save() {
    if (!tree || readOnly) return;
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

  // Dirtiness per group, so the hint can name where the unsaved edits live when the operator has
  // navigated away from them. The maintenance group's edits belong to the Maintenance section.
  const originalTree: FieldRecord | null = original ? (JSON.parse(original) as FieldRecord) : null;
  const dirtyGroups =
    tree && originalTree
      ? Object.keys(tree as unknown as FieldRecord).filter(
          (group) =>
            JSON.stringify((tree as unknown as FieldRecord)[group]) !==
            JSON.stringify(originalTree[group]),
        )
      : [];
  const dirty = dirtyGroups.length > 0;
  const sectionOf = (group: string) => (group === "maintenance" ? "maintenance" : group);
  const hiddenDirty = dirtyGroups.filter((group) => sectionOf(group) !== section);

  return (
    <div>
      {/* The save control sits at the view's top right, the status hint to its left — fixed in place,
          so it never moves with the height of the open section. */}
      <div className="mb-5 flex items-center justify-end gap-4">
        {status === "error" && error && <Hint tone="error">{error}</Hint>}
        {readOnly ? (
          <Hint className="text-2xs">read-only — settings cannot be saved in inspection mode</Hint>
        ) : (
          <>
            {!dirty && status === "ready" && <Hint>no unsaved changes</Hint>}
            {dirty && status !== "saving" && hiddenDirty.length > 0 && (
              <Hint>
                unsaved changes in {hiddenDirty.map((group) => title(label(group))).join(", ")}
              </Hint>
            )}
          </>
        )}
        <Button primary onClick={save} disabled={!dirty || readOnly || status === "saving"}>
          {status === "saving" ? "Saving…" : "Save"}
        </Button>
      </div>
      <div className="grid grid-cols-1 gap-5 md:grid-cols-[8rem_1fr] md:gap-8">
        <div className="self-start">
          <SectionSelect sections={sections} selected={section} onSelect={selectSection} />
          <div className="hidden md:block">
            <SectionNav sections={sections} selected={section} />
          </div>
        </div>

        <div>
          {section === "environment" ? (
            <EnvironmentSection connection={connection} />
          ) : section === "maintenance" ? (
            <div className="flex flex-col gap-8">
              {tree && (
                <BehavioralSettings
                  tree={tree}
                  group="maintenance"
                  status={status}
                  error={error}
                  onChange={update}
                />
              )}
              <MaintenanceSection connection={connection} events={events} readOnly={readOnly} />
            </div>
          ) : (
            <BehavioralSettings
              tree={tree}
              group={section}
              status={status}
              error={error}
              onChange={update}
            />
          )}
        </div>
      </div>
    </div>
  );
}

/// The desktop sidebar list of sections, mirroring the State and Prompts views' lists: a left-aligned
/// column of buttons, the selected one marked by a clay left-rule.
function SectionNav({
  sections,
  selected,
}: {
  sections: { id: SectionId; label: string }[];
  selected: SectionId;
}) {
  const { link } = useStream();
  return (
    <nav className="flex flex-col">
      {sections.map((entry) => {
        const active = entry.id === selected;
        return (
          <Link
            key={entry.id}
            to={link.settings(entry.id)}
            className={
              "-ml-3 flex w-full min-w-0 items-baseline border-l-2 py-1 pl-2.5 text-left transition-colors " +
              (active ? "border-clay text-ink" : "border-transparent text-ink-soft hover:text-ink")
            }
          >
            <span className="truncate font-mono text-xs">{entry.label}</span>
          </Link>
        );
      })}
    </nav>
  );
}

/// The mobile face of the section list: a native dropdown, so the opened section owns the screen
/// instead of scrolling past the whole list. Hidden once there is room for the sidebar (`md`).
function SectionSelect({
  sections,
  selected,
  onSelect,
}: {
  sections: { id: SectionId; label: string }[];
  selected: SectionId;
  onSelect: (id: string) => void;
}) {
  return (
    <Select
      value={selected}
      onChange={(event) => onSelect(event.target.value)}
      className="md:hidden"
      aria-label="Choose a settings section"
    >
      {sections.map((entry) => (
        <option key={entry.id} value={entry.id}>
          {entry.label}
        </option>
      ))}
    </Select>
  );
}

export default SettingsView;
