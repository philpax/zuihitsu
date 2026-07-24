import { useEffect, useState } from "react";

import type { Event } from "@zuihitsu/wire/types/Event.ts";
import type { PromptTemplateName } from "@zuihitsu/wire/types/PromptTemplateName.ts";
import type { LiveConnection } from "../../lib/api/live.ts";
import {
  type PromptTemplate,
  type TemplateStatus,
  deriveTemplates,
  getTemplateStatuses,
  registerPrompt,
} from "../../lib/api/prompts.ts";
import { Button, Hint, Segmented } from "../../components/primitives.tsx";

/// The Prompts view: the agent's prompt templates — the system-prompt scaffold and the framing
/// templates — read from the log and editable (spec §Initialization → prompt templates). A save
/// registers a new version under operator authority; the old version stays on the log, so past
/// `produced_by` references keep resolving and the change shows in the Events view. The bodies are
/// the *definitions*; the assembled prompt each call actually saw is in the Conversation deliberation.
///
/// A curated (operator-edited) template whose build default has since moved on is badged with the
/// newer default's version: an unchanged default auto-tracks the build at boot, but an operator-edited
/// surface is sovereign and adopts a new default only on the operator's explicit choice
/// (`debug upgrade-prompts --force`). The build defaults live in Rust, so the badge state is fetched
/// from `/control/prompt-status` rather than derived from the event log.
export function PromptsView({
  connection,
  events,
}: {
  connection: LiveConnection;
  events: Event[];
}) {
  const templates = deriveTemplates(events, Number.MAX_SAFE_INTEGER);
  const [selected, setSelected] = useState<PromptTemplateName | null>(null);
  const [statuses, setStatuses] = useState<Map<PromptTemplateName, TemplateStatus>>(new Map());

  useEffect(() => {
    let cancelled = false;
    getTemplateStatuses(connection).then(
      (list) => {
        if (cancelled) return;
        setStatuses(new Map(list.map((status) => [status.name, status])));
      },
      () => !cancelled && setStatuses(new Map()),
    );
    return () => {
      cancelled = true;
    };
  }, [connection]);

  if (templates.length === 0) {
    return (
      <div className="py-16 text-center text-sm text-ink-faint">
        No prompt templates registered yet.
      </div>
    );
  }

  const active = templates.find((template) => template.name === selected) ?? templates[0];
  return (
    <div className="mx-auto max-w-3xl">
      <Segmented
        options={templates.map((template) => ({
          id: template.name,
          // A caret marks a curated template whose build default has moved on, so an upgrade is
          // noticeable across the tabs without opening each one.
          label: statuses.get(template.name)?.upgrade_available
            ? `${template.name} ↑`
            : template.name,
        }))}
        value={active.name}
        onChange={(id) => setSelected(id as PromptTemplateName)}
        className="mb-6"
      />

      {/* Keyed by name so selecting a different template remounts the editor with a fresh draft; a
          new version of the *same* template (arriving via the tail after a save) keeps the key, so it
          does not clobber what is being typed. */}
      <PromptEditor
        key={active.name}
        template={active}
        status={statuses.get(active.name)}
        connection={connection}
      />
    </div>
  );
}

function PromptEditor({
  template,
  status,
  connection,
}: {
  template: PromptTemplate;
  status: TemplateStatus | undefined;
  connection: LiveConnection;
}) {
  const [draft, setDraft] = useState(template.body);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const dirty = draft !== template.body;

  async function save() {
    setSaving(true);
    setError(null);
    try {
      await registerPrompt(connection, template.name, draft);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setSaving(false);
    }
  }

  return (
    <>
      <p className="mb-2 font-mono text-2xs tracking-widest text-ink-faint uppercase">
        {template.name} · version {template.version}
      </p>
      {status?.upgrade_available && (
        <p className="mb-2 text-xs text-ink-faint">
          Updated default available (v{status.default_version}). This is an operator-edited surface,
          so it stays as you left it; adopt the new default with{" "}
          <code className="font-mono">debug upgrade-prompts --force</code>.
        </p>
      )}
      <textarea
        value={draft}
        onChange={(event) => setDraft(event.target.value)}
        rows={18}
        spellCheck={false}
        className="w-full resize-y rounded-xs border border-line bg-paper-raised p-3 font-mono text-xs/relaxed text-ink focus:border-ink-faint focus:outline-none"
      />
      <div className="mt-3 flex items-center gap-4">
        <Button primary onClick={save} disabled={!dirty || saving}>
          {saving ? "Saving…" : `Save as version ${template.version + 1}`}
        </Button>
        {dirty && (
          <button
            onClick={() => setDraft(template.body)}
            className="font-mono text-xs text-ink-faint transition-colors hover:text-clay"
          >
            revert
          </button>
        )}
        {error && <Hint tone="error">{error}</Hint>}
      </div>
    </>
  );
}
