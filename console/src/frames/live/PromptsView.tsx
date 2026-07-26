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
import { useNavigate } from "../../lib/nav/historyContext.ts";
import { useStream } from "../../lib/nav/useStreamLocation.ts";
import { Button, Hint, Select } from "../../components/primitives.tsx";
import { useReadOnly } from "../../lib/view/readOnly.ts";

/// The Prompts view: the agent's prompt templates — the system-prompt scaffold and the framing
/// templates — read from the log and editable (spec §Initialization → prompt templates). A save
/// registers a new version under operator authority; the old version stays on the log, so past
/// `produced_by` references keep resolving and the change shows in the Events view. The bodies are
/// the *definitions*; the assembled prompt each call actually saw is in the Conversation deliberation.
///
/// Laid out as a master-detail pair, the same motif as the State view: a left-aligned list of the
/// templates, the selected one's body in the detail pane to its right. The selected template rides in
/// the URL as the view's selection segment (`/live/prompts/<template-name>`), so a template is a
/// shareable deep link and browser back and forward walk the selection; a segment that names no
/// registered template (a renamed or dropped one) falls back to the first template.
///
/// A curated (operator-edited) template whose build default has since moved on is badged with the
/// newer default's version: an unchanged default auto-tracks the build at boot, but an operator-edited
/// surface is sovereign and adopts a new default only on the operator's explicit choice
/// (`debug upgrade-prompts --force`). The build defaults live in Rust, so the badge state is fetched
/// from `/control/prompt-status` rather than derived from the event log.
///
/// On a read-only instance the bodies still read and still take edits, but registering a version is
/// refused with a `409`, so the save is held closed with a note saying why.
export function PromptsView({
  connection,
  events,
}: {
  connection: LiveConnection;
  events: Event[];
}) {
  const templates = deriveTemplates(events, Number.MAX_SAFE_INTEGER);
  const navigate = useNavigate();
  const { selection, link } = useStream();
  const [statuses, setStatuses] = useState<Map<PromptTemplateName, TemplateStatus>>(new Map());

  // Selecting a template is navigation (a pushed history entry): the selection rides the URL segment.
  const selectTemplate = (name: PromptTemplateName) =>
    navigate(link.view("prompts", { selection: name }));

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

  const active = templates.find((template) => template.name === selection) ?? templates[0];
  return (
    <div className="grid grid-cols-1 gap-5 md:grid-cols-[15rem_1fr] md:gap-8">
      <div className="flex flex-col gap-4 self-start">
        <PromptSelect
          templates={templates}
          statuses={statuses}
          selected={active.name}
          onSelect={selectTemplate}
        />
        <div className="hidden md:block">
          <PromptList
            templates={templates}
            statuses={statuses}
            selected={active.name}
            onSelect={selectTemplate}
          />
        </div>
      </div>

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

/// The sidebar list of templates, mirroring the State view's memory list: a left-aligned column of
/// buttons, the selected one marked by a clay rule. A caret marks a curated template whose build
/// default has moved on, so a pending upgrade is noticeable across the list without opening each one.
function PromptList({
  templates,
  statuses,
  selected,
  onSelect,
}: {
  templates: PromptTemplate[];
  statuses: Map<PromptTemplateName, TemplateStatus>;
  selected: PromptTemplateName;
  onSelect: (name: PromptTemplateName) => void;
}) {
  return (
    <nav className="flex flex-col">
      {templates.map((template) => {
        const active = template.name === selected;
        const description = statuses.get(template.name)?.description;
        return (
          <button
            key={template.name}
            onClick={() => onSelect(template.name)}
            title={description ? `${template.name} · ${description}` : template.name}
            className={
              "-ml-3 flex w-full min-w-0 items-baseline border-l-2 py-1 pl-2.5 text-left transition-colors " +
              (active ? "border-clay text-ink" : "border-transparent text-ink-soft hover:text-ink")
            }
          >
            <span className="truncate font-mono text-xs">{template.name}</span>
            {statuses.get(template.name)?.upgrade_available && (
              <span className="ml-1.5 shrink-0 text-clay" title="an updated default is available">
                ↑
              </span>
            )}
          </button>
        );
      })}
    </nav>
  );
}

/// The mobile face of the template list: a native dropdown, so the opened template owns the screen
/// instead of scrolling past the whole list. Hidden once there is room for the sidebar (`md`).
function PromptSelect({
  templates,
  statuses,
  selected,
  onSelect,
}: {
  templates: PromptTemplate[];
  statuses: Map<PromptTemplateName, TemplateStatus>;
  selected: PromptTemplateName;
  onSelect: (name: PromptTemplateName) => void;
}) {
  return (
    <Select
      value={selected}
      onChange={(event) => onSelect(event.target.value as PromptTemplateName)}
      className="md:hidden"
      aria-label="Choose a prompt template"
    >
      {templates.map((template) => (
        <option key={template.name} value={template.name}>
          {template.name}
          {statuses.get(template.name)?.upgrade_available ? " ↑" : ""}
        </option>
      ))}
    </Select>
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
  const readOnly = useReadOnly();
  const [draft, setDraft] = useState(template.body);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const dirty = draft !== template.body;

  async function save() {
    if (readOnly) return;
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
    <div>
      <p className="mb-2 font-mono text-2xs tracking-widest text-ink-faint uppercase">
        {template.name} · version {template.version}
      </p>
      {status?.description && <p className="mb-2 text-xs text-ink-faint">{status.description}</p>}
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
        <Button primary onClick={save} disabled={!dirty || readOnly || saving}>
          {saving ? "Saving…" : `Save as version ${template.version + 1}`}
        </Button>
        {readOnly && (
          <Hint className="text-2xs">
            read-only — a new version cannot be registered in inspection mode
          </Hint>
        )}
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
    </div>
  );
}
