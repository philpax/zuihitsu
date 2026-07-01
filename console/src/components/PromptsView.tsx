import { useState } from "react";

import type { Event } from "../types/Event.ts";
import type { PromptTemplateName } from "../types/PromptTemplateName.ts";
import type { LiveConnection } from "../lib/live.ts";
import { type PromptTemplate, deriveTemplates, registerPrompt } from "../lib/prompts.ts";
import { Button, Hint, Segmented } from "./primitives.tsx";

/// The Prompts view: the agent's prompt templates — the system-prompt scaffold and the framing
/// templates — read from the log and editable (spec §Initialization → prompt templates). A save
/// registers a new version under operator authority; the old version stays on the log, so past
/// `produced_by` references keep resolving and the change shows in the Events view. The bodies are
/// the *definitions*; the assembled prompt each call actually saw is in the Conversation deliberation.
export function PromptsView({
  connection,
  events,
}: {
  connection: LiveConnection;
  events: Event[];
}) {
  const templates = deriveTemplates(events, Number.MAX_SAFE_INTEGER);
  const [selected, setSelected] = useState<PromptTemplateName | null>(null);

  if (templates.length === 0) {
    return (
      <div className="py-16 text-center text-sm text-ink-faint">
        No prompt templates registered yet.
      </div>
    );
  }

  const active = templates.find((template) => template.name === selected) ?? templates[0];
  return (
    <div>
      <Segmented
        options={templates.map((template) => ({ id: template.name, label: template.name }))}
        value={active.name}
        onChange={(id) => setSelected(id as PromptTemplateName)}
        className="mb-6"
      />

      {/* Keyed by name so selecting a different template remounts the editor with a fresh draft; a
          new version of the *same* template (arriving via the tail after a save) keeps the key, so it
          does not clobber what is being typed. */}
      <PromptEditor key={active.name} template={active} connection={connection} />
    </div>
  );
}

function PromptEditor({
  template,
  connection,
}: {
  template: PromptTemplate;
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
      <p className="mb-2 font-mono text-2xs uppercase tracking-widest text-ink-faint">
        {template.name} · version {template.version}
      </p>
      <textarea
        value={draft}
        onChange={(event) => setDraft(event.target.value)}
        rows={18}
        spellCheck={false}
        className="w-full resize-y rounded-xs border border-line bg-paper-raised p-3 font-mono text-xs leading-relaxed text-ink focus:border-ink-faint focus:outline-none"
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
