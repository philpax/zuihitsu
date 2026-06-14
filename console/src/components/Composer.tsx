import { useState } from "react";

/// A message composer: a growing input that sends on Enter (Shift+Enter for a newline), with a
/// pending state while the turn runs and any failure surfaced inline. `onSend` runs the turn — the
/// caller chooses the endpoint and authority (a participant message or an operator imprint) — and
/// the reply arrives through the live tail.
export function Composer({
  onSend,
  placeholder = "Write to the agent…",
  disabled = false,
  disabledHint,
}: {
  onSend: (text: string) => Promise<void>;
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
    setPending(true);
    setError(null);
    try {
      await onSend(text);
      setDraft("");
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setPending(false);
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
        placeholder={disabled ? disabledHint : pending ? "The agent is thinking…" : placeholder}
        disabled={pending || disabled}
        className="w-full resize-none bg-transparent font-serif text-base leading-relaxed text-ink placeholder:text-ink-faint/60 focus:outline-none disabled:opacity-60"
      />
      <div className="mt-2 flex items-center justify-between">
        {error ? (
          <span className="font-mono text-2xs text-clay">{error}</span>
        ) : (
          <span className="font-mono text-2xs text-ink-faint">
            enter to send · shift+enter for a newline
          </span>
        )}
        <button
          onClick={send}
          disabled={pending || disabled || draft.trim().length === 0}
          className="border border-line-strong px-4 py-1.5 font-mono text-xs text-ink transition-colors enabled:hover:border-clay enabled:hover:text-clay disabled:opacity-45"
        >
          {pending ? "…" : "send"}
        </button>
      </div>
    </div>
  );
}
