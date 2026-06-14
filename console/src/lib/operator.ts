import type { LiveConnection } from "./live.ts";
import { authHeaders, errorMessage } from "./http.ts";

/// What boot found in the agent's log: no events yet, an interrupted genesis to re-drive, or a born
/// agent ready to serve (mirrors the Rust `GenesisStatus`, which serializes as this bare string).
export type GenesisStatus = "Empty" | "Incomplete" | "Complete";

/// The seed an operator gives a new agent: its name, a one-line persona, and any first-person
/// entries to plant in `self` (mirrors the Rust `SeedSelf`).
export interface Seed {
  agent_name: string;
  persona: string;
  seed_entries: string[];
}

/// Whether the agent exists and is ready. The Operator view gates on this: create the agent when
/// `Empty`, otherwise run the imprint interview.
export async function genesisStatus(connection: LiveConnection): Promise<GenesisStatus> {
  const response = await fetch(`${connection.baseUrl}/control/genesis`, {
    headers: authHeaders(connection),
  });
  if (!response.ok) throw new Error(await errorMessage(response));
  return (await response.json()) as GenesisStatus;
}

/// Create the agent — or resume an interrupted genesis (idempotent on a born agent). The new events
/// surface through the live tail like any others.
export async function createAgent(connection: LiveConnection, seed: Seed): Promise<void> {
  const response = await fetch(`${connection.baseUrl}/control/agent`, {
    method: "POST",
    headers: authHeaders(connection),
    body: JSON.stringify(seed),
  });
  if (!response.ok) throw new Error(await errorMessage(response));
}

/// Deliver one operator message of the imprint interview and run the agent's response — the only
/// path that may write `self`. The turns it produces (the operator's message and the agent's reply)
/// arrive through the live tail. Throws with the agent's reason on failure (e.g. no model configured).
export async function imprint(connection: LiveConnection, text: string): Promise<void> {
  const response = await fetch(`${connection.baseUrl}/control/imprint`, {
    method: "POST",
    headers: authHeaders(connection),
    body: JSON.stringify({ text }),
  });
  if (!response.ok) throw new Error(await errorMessage(response));
}
