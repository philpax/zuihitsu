import { useEffect, useState } from "react";

import type { Event } from "../types/Event.ts";
import type { Replica } from "../lib/replica.ts";
import type { LiveConnection } from "../lib/live.ts";
import {
  type GenesisStatus,
  type Seed,
  createAgent,
  genesisStatus,
  imprint,
} from "../lib/operator.ts";
import { type TurnModel, buildConversations } from "../lib/conversation.ts";
import { nameById } from "../lib/labels.ts";
import { Eyebrow } from "./primitives.tsx";
import { OutcomeList } from "./OutcomeList.tsx";

/// The Operator view: the one surface where the two frames genuinely diverge, so it lives only in
/// the agent frame. Under operator authority the console may speak to the agent directly — the
/// imprint interview, the only path that may write `self`. When no agent exists yet, it hosts the
/// genesis form instead; once born, it is a calm chat whose turns flow back through the live tail.
export function OperatorChat({
  replica,
  events,
  connection,
}: {
  replica: Replica;
  events: Event[];
  connection: LiveConnection;
}) {
  const [status, setStatus] = useState<GenesisStatus | "loading" | "unreachable">("loading");

  useEffect(() => {
    let cancelled = false;
    genesisStatus(connection).then(
      (value) => !cancelled && setStatus(value),
      () => !cancelled && setStatus("unreachable"),
    );
    return () => {
      cancelled = true;
    };
  }, [connection]);

  return (
    <div className="mx-auto max-w-prose">
      <header className="mb-8">
        <h2 className="font-serif text-2xl text-ink">Operator</h2>
        <p className="mt-1 max-w-prose text-sm leading-relaxed text-ink-soft">
          Speak to the agent under operator authority — the imprint interview, where it learns who
          you are and what it is for. This is the only path that may write <code>self</code>.
        </p>
      </header>

      {status === "loading" ? (
        <p className="py-16 text-center text-sm text-ink-faint">Checking the agent…</p>
      ) : status === "unreachable" ? (
        <p className="py-16 text-center text-sm text-clay">Could not reach the agent.</p>
      ) : status === "Complete" ? (
        <ImprintChat replica={replica} events={events} connection={connection} />
      ) : (
        <GenesisForm
          connection={connection}
          resuming={status === "Incomplete"}
          onCreated={() => setStatus("Complete")}
        />
      )}
    </div>
  );
}

/// Bring the agent into being: name it, give it a one-line persona, and plant any first-person seed
/// entries in `self`. Shown when the connected instance has no agent yet (or an interrupted genesis).
function GenesisForm({
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
        <input
          value={persona}
          onChange={(event) => setPersona(event.target.value)}
          placeholder="A thoughtful, discreet companion with a long memory."
          className="w-full border-b border-line bg-transparent pb-1.5 text-base text-ink placeholder:text-ink-faint/60 focus:border-ink-faint focus:outline-none"
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
  );
}

/// The imprint chat: the operator/imprint conversation rendered as turns, with a composer. Sending
/// runs a turn on the agent; its reply arrives through the live tail, so the transcript fills in on
/// its own a beat later.
function ImprintChat({
  replica,
  events,
  connection,
}: {
  replica: Replica;
  events: Event[];
  connection: LiveConnection;
}) {
  const [draft, setDraft] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const imprintRoom = buildConversations(events, nameById(replica.memories(""))).find(
    (conversation) => conversation.platform === "operator",
  );
  const turns = (imprintRoom?.turns ?? []).filter((turn) => turn.role !== "System");

  async function send() {
    const text = draft.trim();
    if (!text || pending) return;
    setPending(true);
    setError(null);
    try {
      await imprint(connection, text);
      setDraft("");
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setPending(false);
    }
  }

  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-col gap-4">
        {turns.length === 0 ? (
          <p className="py-10 text-center text-sm text-ink-faint">
            Introduce yourself to begin the interview.
          </p>
        ) : (
          turns.map((turn) => <Bubble key={turn.turnId} turn={turn} />)
        )}
      </div>

      <div className="border-t border-line pt-5">
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
          placeholder={pending ? "The agent is thinking…" : "Write to the agent…"}
          disabled={pending}
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
            disabled={pending || draft.trim().length === 0}
            className="border border-line-strong px-4 py-1.5 font-mono text-xs text-ink transition-colors enabled:hover:border-clay enabled:hover:text-clay disabled:opacity-45"
          >
            {pending ? "…" : "send"}
          </button>
        </div>
      </div>
    </div>
  );
}

/// One turn in the interview — the operator's message ranged right, the agent's reply left, with the
/// faint trail of what the reply wrote (a `self` entry, a link) so its learning is visible.
function Bubble({ turn }: { turn: TurnModel }) {
  const isAgent = turn.role === "Agent";
  return (
    <div className={"flex flex-col gap-1 " + (isAgent ? "items-start" : "items-end")}>
      <Eyebrow>{isAgent ? "the agent" : (turn.speaker ?? "operator")}</Eyebrow>
      <div
        className={
          "max-w-[85%] px-4 py-2.5 text-base leading-relaxed " +
          (isAgent ? "border border-line bg-oat/40 text-ink" : "bg-clay-soft/25 text-ink")
        }
      >
        {turn.text || <span className="italic text-ink-faint">stayed silent</span>}
      </div>
      {isAgent && turn.outcomes.length > 0 && (
        <OutcomeList outcomes={turn.outcomes} className="mt-0.5 gap-0.5" />
      )}
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
