import { useState } from "react";

import { Button, Hint } from "./primitives.tsx";

/// A message composer: a growing input that sends on Enter (Shift+Enter for a newline), with a
/// pending state while the turn runs and any failure surfaced inline. `onSend` runs the turn — the
/// caller chooses the endpoint and authority (a participant message or an operator imprint) — and
/// the reply arrives through the live tail. `onPendingChange` lets the conversation show that the
/// agent is working while the turn is in flight.
export function Composer({
  onSend,
  onPendingChange,
  placeholder = "Write to the agent…",
  disabled = false,
  disabledHint,
}: {
  onSend: (text: string) => Promise<void>;
  onPendingChange?: (pending: boolean) => void;
  placeholder?: string;
  disabled?: boolean;
  disabledHint?: string;
}) {
  const [draft, setDraft] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function send() {
    const text = draft.trim();
    if (!text || pending || disabled) return;
    // Clear the box at once, so it does not sit showing the sent text while the agent works; a failed
    // send restores it below, so nothing is lost.
    setDraft("");
    setError(null);
    setPending(true);
    onPendingChange?.(true);
    try {
      await onSend(text);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
      setDraft(text);
    } finally {
      setPending(false);
      onPendingChange?.(false);
    }
  }

  return (
    <div className="border-t border-line pt-4">
      <textarea
        value={draft}
        onChange={(event) => setDraft(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter" && !event.shiftKey) {
            event.preventDefault();
            send();
          }
        }}
        rows={2}
        placeholder={
          disabled ? disabledHint : pending ? "Waiting for the agent's reply…" : placeholder
        }
        disabled={pending || disabled}
        className="w-full resize-none bg-transparent text-base leading-relaxed text-ink placeholder:text-ink-faint/60 focus:outline-none disabled:opacity-60"
      />
      <div className="mt-2 flex items-center justify-between">
        {error ? (
          <Hint tone="error">{error}</Hint>
        ) : (
          <Hint className="hidden sm:inline">enter to send · shift+enter for a newline</Hint>
        )}
        <Button primary onClick={send} disabled={pending || disabled || draft.trim().length === 0}>
          {pending ? "…" : "send"}
        </Button>
      </div>
    </div>
  );
}
