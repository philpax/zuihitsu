import { useState } from "react";

import type { LiveConnection } from "../lib/live.ts";
import { type Seed, createAgent } from "../lib/operator.ts";
import { Eyebrow } from "./primitives.tsx";

/// Bring the agent into being before the workspace opens: name it, give it a persona, and
/// plant any first-person seed entries in `self`. Shown by the agent frame when the connected
/// instance has no agent yet (or an interrupted genesis to resume) — the one operator action that
/// gates everything else, so it stands ahead of the views rather than inside them.
export function GenesisGate({
  connection,
  resuming,
  onCreated,
}: {
  connection: LiveConnection;
  resuming: boolean;
  onCreated: () => void;
}) {
  const [name, setName] = useState("");
  const [persona, setPersona] = useState("");
  const [seeds, setSeeds] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit() {
    setPending(true);
    setError(null);
    const seed: Seed = {
      agent_name: name.trim(),
      persona: persona.trim(),
      seed_entries: seeds
        .split("\n")
        .map((line) => line.trim())
        .filter(Boolean),
    };
    try {
      await createAgent(connection, seed);
      onCreated();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
      setPending(false);
    }
  }

  const ready = name.trim().length > 0 && persona.trim().length > 0;
  return (
    <div className="mx-auto max-w-prose py-12">
      <header className="mb-6">
        <h2 className="font-serif text-xl text-ink sm:text-2xl">No agent here yet</h2>
        <p className="mt-1 max-w-prose text-sm leading-relaxed text-ink-soft">
          This instance has no agent. Bring one into being — its name, a persona, and any
          first-person truths to plant in <code>self</code>. You can refine who it is afterward in
          the imprint conversation.
        </p>
      </header>

      <div className="flex flex-col gap-6">
        {resuming && (
          <p className="text-sm text-clay">
            A genesis was started but not completed — submitting resumes it.
          </p>
        )}
        <Field label="name">
          <input
            value={name}
            onChange={(event) => setName(event.target.value)}
            placeholder="Kestrel"
            className="w-full border-b border-line bg-transparent pb-1.5 text-base text-ink placeholder:text-ink-faint/60 focus:border-ink-faint focus:outline-none"
          />
        </Field>
        <Field label="persona">
          <textarea
            value={persona}
            onChange={(event) => setPersona(event.target.value)}
            rows={3}
            placeholder="A thoughtful, discreet companion with a long memory."
            className="w-full resize-none border border-line bg-transparent p-3 font-serif text-base leading-relaxed text-ink placeholder:text-ink-faint/60 focus:border-ink-faint focus:outline-none"
          />
        </Field>
        <Field label="seed entries · one per line">
          <textarea
            value={seeds}
            onChange={(event) => setSeeds(event.target.value)}
            rows={3}
            placeholder="I keep what people tell me in confidence."
            className="w-full resize-none border border-line bg-transparent p-3 font-serif text-base leading-relaxed text-ink placeholder:text-ink-faint/60 focus:border-ink-faint focus:outline-none"
          />
        </Field>
        {error && <p className="text-sm text-clay">{error}</p>}
        <button
          onClick={submit}
          disabled={!ready || pending}
          className="self-start border border-line-strong px-5 py-2 text-base text-ink transition-colors enabled:hover:border-clay enabled:hover:text-clay disabled:opacity-45"
        >
          {pending ? "Creating…" : resuming ? "Resume genesis" : "Create the agent"}
        </button>
      </div>
    </div>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="flex flex-col gap-2">
      <Eyebrow>{label}</Eyebrow>
      {children}
    </label>
  );
}
