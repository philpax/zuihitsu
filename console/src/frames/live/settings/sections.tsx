import { useEffect, useState } from "react";

import type { Event } from "@zuihitsu/wire/types/Event.ts";
import type { LiveConnection } from "../../../lib/api/live.ts";
import { maintenancePassLabel } from "../../../lib/model/events.ts";
import { formatDateTime } from "../../../lib/format/format.ts";
import { useNavigate } from "../../../lib/nav/historyContext.ts";
import { useStream } from "../../../lib/nav/useStreamLocation.ts";
import { type Settings, getSettings, putSettings } from "../../../lib/api/settings.ts";
import { type ConfigTree, getConfig } from "../../../lib/api/config.ts";
import { snapshotNow } from "../../../lib/api/operator.ts";
import { Button, Eyebrow, Hint, Segmented } from "../../../components/primitives.tsx";
import { type FieldRecord, type FieldValue, label, setIn } from "./settingsUtilities.ts";
import { ConfigFields, Fields } from "./fields.tsx";

export { SECTIONS, type SectionId } from "./sectionConstants.ts";
import { SECTIONS, type SectionId } from "./sectionConstants.ts";

/// The Settings view: the agent's behavioral settings (the latest `ConfigSet` snapshot), read and
/// edited live (spec §Initialization → configuration). A save logs a new operator `ConfigSet` that
/// takes effect on the next read, so the change shows up in the Events view and time-travels like
/// anything else.
export function SettingsView({
  connection,
  events,
}: {
  connection: LiveConnection;
  events: Event[];
}) {
  const navigate = useNavigate();
  const { selection, link } = useStream();
  // The open tab rides the selection segment; a bare `/…/settings` defaults to the behavioral
  // settings tab.
  const section: SectionId = SECTIONS.some((entry) => entry.id === selection)
    ? (selection as SectionId)
    : "settings";

  // Switching tabs is navigation, so it pushes a history entry (back returns to the prior tab).
  function selectSection(id: string) {
    navigate(link.settings(id));
  }

  return (
    <div className="mx-auto max-w-2xl">
      <Segmented options={SECTIONS} value={section} onChange={selectSection} className="mb-6" />
      {section === "settings" && <BehavioralSettings connection={connection} />}
      {section === "environment" && <EnvironmentSection connection={connection} />}
      {section === "maintenance" && <MaintenanceSection connection={connection} events={events} />}
    </div>
  );
}

/// The editable behavioral settings tree, with the save bar footing it.
export function BehavioralSettings({ connection }: { connection: LiveConnection }) {
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

      <div className="sticky bottom-0 mt-8 flex items-center gap-4 border-t border-line bg-paper/95 py-4 backdrop-blur-sm">
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
export function EnvironmentSection({ connection }: { connection: LiveConnection }) {
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
      <p className="max-w-prose text-sm/relaxed text-ink-soft">
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

/// A tick interval in readable units: seconds under a minute, whole minutes under an hour, hours
/// beyond — the cadence line's register, not a general duration formatter.
function formatSeconds(seconds: number): string {
  if (seconds < 60) return `${seconds}s`;
  if (seconds < 3600) return `${Math.round(seconds / 60)}m`;
  return `${(seconds / 3600).toFixed(1)}h`;
}

/// Maintenance actions against the running instance, and the history of autonomous maintenance
/// sweeps. The history folds every `MaintenancePassCompleted` on the log — the sweep-level record each
/// pass driver appends — so the operator can see when a pass ran, over what log window, and how much it
/// did. Cursor-free like `PromptsView`: the maintenance subtab is an operator control surface reporting
/// current standing, not part of the time-travelling deliberation timeline, so it reads the whole log
/// rather than filtering by the viewer's seq cursor.
export function MaintenanceSection({
  connection,
  events,
}: {
  connection: LiveConnection;
  events: Event[];
}) {
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

  // Newest first: the maintenance passes' sweep records, folded from the whole log.
  const sweeps = events
    .filter((event) => event.payload.type === "MaintenancePassCompleted")
    .reverse();

  // The driver's cadence, from the latest ConfigSet on the log. The ticker itself is anchored at
  // boot in memory, so an exact next-fire time is not derivable from the log — the honest statement
  // is the interval and the activity gates, not a countdown.
  let cadence: { enabled: boolean; tickSeconds: number; gates: string } | null = null;
  for (const event of events) {
    if (event.payload.type !== "ConfigSet") continue;
    const m = event.payload.settings.maintenance;
    const gates = [
      m.consolidation_min_activity,
      m.canonicalize_min_activity,
      m.link_cleanup_min_activity,
    ];
    cadence = {
      enabled: m.enabled,
      tickSeconds: m.tick_seconds,
      gates: gates.every((gate) => gate === gates[0])
        ? `${gates[0]}`
        : gates.map((gate) => `${gate}`).join("/"),
    };
  }

  return (
    <div className="flex flex-col gap-10">
      <section>
        <Eyebrow>Graph snapshot</Eyebrow>
        <p className="mt-3 max-w-prose text-sm/relaxed text-ink-soft">
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

      <section>
        <Eyebrow>Maintenance passes</Eyebrow>
        <p className="mt-3 max-w-prose text-sm/relaxed text-ink-soft">
          The autonomous data-hygiene sweeps — consolidation, canonicalize, and link cleanup — that
          run off the hot path. Each sweep records when it ran, the log window it swept, and how
          many effects it committed.
        </p>
        {cadence && (
          <p className="mt-2 font-mono text-xs text-ink-faint">
            {cadence.enabled
              ? `ticks every ${formatSeconds(cadence.tickSeconds)} · a pass sweeps once ${cadence.gates} events have accrued since its last run`
              : "timer disabled — passes run only on demand"}
          </p>
        )}
        {sweeps.length === 0 ? (
          <p className="mt-4 text-sm text-ink-faint">No maintenance sweeps recorded yet.</p>
        ) : (
          <ul className="mt-4 flex flex-col divide-y divide-line">
            {sweeps.map((event) => {
              // Narrowed by the filter above; the guard keeps TypeScript honest.
              if (event.payload.type !== "MaintenancePassCompleted") return null;
              const { pass, from, to, actions } = event.payload;
              return (
                <li
                  key={event.seq}
                  className="flex items-baseline justify-between gap-4 py-2 text-sm"
                >
                  <span className="text-ink">{maintenancePassLabel(pass)}</span>
                  <span className="font-mono text-xs text-ink-faint">
                    seq {from}–{to} · {actions} {actions === 1 ? "action" : "actions"}
                  </span>
                  <span className="shrink-0 text-xs text-ink-faint">
                    {formatDateTime(event.recorded_at)}
                  </span>
                </li>
              );
            })}
          </ul>
        )}
      </section>
    </div>
  );
}
