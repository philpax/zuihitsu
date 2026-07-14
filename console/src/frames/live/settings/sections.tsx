import { useEffect, useState } from "react";
import { useSearchParams } from "react-router-dom";

import type { LiveConnection } from "../../../lib/api/live.ts";
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
    <div className="mx-auto max-w-2xl">
      <Segmented options={SECTIONS} value={section} onChange={selectSection} className="mb-6" />
      {section === "settings" && <BehavioralSettings connection={connection} />}
      {section === "environment" && <EnvironmentSection connection={connection} />}
      {section === "maintenance" && <MaintenanceSection connection={connection} />}
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
export function MaintenanceSection({ connection }: { connection: LiveConnection }) {
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
