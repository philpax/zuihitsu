import { useState } from "react";

import type { EntryId } from "../../types/EntryId.ts";
import type { EntryView } from "../../lib/model/graph.ts";
import { Button, Eyebrow, Hint } from "../../components/primitives.tsx";

/// The operator's `self`-editing panel, shown on the `self` memory in the live agent frame at the head.
/// The console-direct counterpart to the imprint interview: where the imprint writes `self` by running
/// the model conversationally, this writes it outright under operator authority — the same authority
/// that edits the scaffold and settings. One form covers both operations: with the target set to "a new
/// entry" it appends a charter entry; with an existing entry chosen it revises it — the new text
/// replaces the old entry, which drops from the live profile while remaining in history. Because
/// revising retires a charter line the system prompt reads verbatim, it takes a deliberate second click.
/// The write's events arrive through the live tail, so the browser re-folds and the panel resets.
export function SelfEditor({
  entries,
  onEditSelf,
}: {
  entries: EntryView[];
  onEditSelf: (text: string, supersedes?: EntryId) => Promise<void>;
}) {
  // The edit target: `"new"` appends, an `entry_id` revises that entry. `text` is the draft; `original`
  // is the chosen entry's text, so a revision can disable Save until the operator actually changes it.
  const [target, setTarget] = useState<string>("new");
  const [text, setText] = useState("");
  const [original, setOriginal] = useState("");
  const [saving, setSaving] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const revising = target !== "new";
  const trimmed = text.trim();
  const unchanged = revising && text === original;
  const ready = trimmed.length > 0 && !unchanged;

  function choose(value: string) {
    setTarget(value);
    setConfirming(false);
    setError(null);
    const chosen = entries.find((entry) => entry.entry_id === value);
    const body = chosen ? chosen.text : "";
    setText(body);
    setOriginal(body);
  }

  async function commit() {
    if (!ready) return;
    setSaving(true);
    setError(null);
    try {
      await onEditSelf(trimmed, revising ? (target as EntryId) : undefined);
      setTarget("new");
      setText("");
      setOriginal("");
      setConfirming(false);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setSaving(false);
    }
  }

  return (
    <section className="mt-8 border-t border-line pt-5">
      <Eyebrow>edit self</Eyebrow>
      <p className="mt-2 font-mono text-2xs text-ink-faint">
        Append a charter entry, or revise one — the operator side of the imprint interview.
      </p>

      <label className="mt-4 flex flex-col gap-1.5">
        <span className="font-mono text-2xs text-ink-faint">target</span>
        <select
          value={target}
          onChange={(event) => choose(event.target.value)}
          className="border border-line bg-transparent px-2 py-1.5 font-mono text-xs text-ink focus:border-ink-faint focus:outline-none"
        >
          <option value="new">a new entry</option>
          {entries.map((entry, index) => (
            <option key={entry.entry_id} value={entry.entry_id}>
              revise #{index + 1}: {summarize(entry.text)}
            </option>
          ))}
        </select>
      </label>

      <textarea
        value={text}
        onChange={(event) => {
          setText(event.target.value);
          setConfirming(false);
        }}
        rows={3}
        placeholder="the agent's self, in its own voice…"
        className="mt-3 w-full resize-y border border-line bg-transparent px-3 py-2 font-serif text-base leading-relaxed text-ink placeholder:text-ink-faint/60 focus:border-ink-faint focus:outline-none"
      />

      <div className="mt-2 flex items-center gap-2">
        {revising && confirming ? (
          <>
            <Hint tone="error">revise this entry? it drops from the live profile.</Hint>
            <Button primary disabled={saving || !ready} onClick={commit}>
              revise
            </Button>
            <Button disabled={saving} onClick={() => setConfirming(false)}>
              cancel
            </Button>
          </>
        ) : (
          <Button
            primary
            disabled={saving || !ready}
            onClick={() => (revising ? setConfirming(true) : commit())}
          >
            {revising ? "revise" : "append entry"}
          </Button>
        )}
        {saving && <Hint>working…</Hint>}
      </div>
      {error && <Hint tone="error">{error}</Hint>}
    </section>
  );
}

/// A one-line preview of an entry for the target dropdown — enough to recognise the line without
/// spilling the whole charter into the selector.
function summarize(text: string): string {
  const line = text.trim().replace(/\s+/g, " ");
  return line.length > 48 ? `${line.slice(0, 47)}…` : line;
}
