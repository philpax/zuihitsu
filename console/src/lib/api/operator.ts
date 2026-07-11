import type { MemoryId } from "../../types/MemoryId.ts";
import type { TurnOutcome } from "../../types/TurnOutcome.ts";
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
/// arrive through the live tail; a `"Deferred"` outcome says the message landed but the model was
/// unreachable, exactly as on the participant path. Throws with the agent's reason on failure
/// (e.g. no model configured).
export async function imprint(connection: LiveConnection, text: string): Promise<TurnOutcome> {
  const response = await fetch(`${connection.baseUrl}/control/imprint`, {
    method: "POST",
    headers: authHeaders(connection),
    body: JSON.stringify({ text }),
  });
  if (!response.ok) throw new Error(await errorMessage(response));
  return (await response.json()) as TurnOutcome;
}

/// Resolve a pending cross-platform merge proposal as the operator (spec §Cross-platform identity →
/// operator-asserted merge). `accept` authors the merging `same_as` between the two stubs — the
/// console-only merge path — while a decline records the operator's refusal so the proposal settles.
/// The resulting event arrives through the live tail, so the derived proposal list updates on the next
/// poll. Throws with the agent's reason on failure.
export async function resolveMerge(
  connection: LiveConnection,
  from: MemoryId,
  to: MemoryId,
  accept: boolean,
): Promise<void> {
  const response = await fetch(`${connection.baseUrl}/control/merge`, {
    method: "POST",
    headers: authHeaders(connection),
    body: JSON.stringify({ from, to, accept }),
  });
  if (!response.ok) throw new Error(await errorMessage(response));
}

/// Retract an operator-asserted `same_as` merge — the undo of `resolveMerge`'s accept (spec
/// §Cross-platform identity → operator-asserted merge). Removes the `same_as` edge between the two
/// stubs, so their visibility classes split back apart on the next fold. The resulting `LinkRemoved`
/// arrives through the live tail, so the derived proposal list re-derives on the next poll. Throws with
/// the agent's reason on failure (a `404` when the pair is not directly merged — nothing to retract).
export async function unmerge(
  connection: LiveConnection,
  from: MemoryId,
  to: MemoryId,
): Promise<void> {
  const response = await fetch(`${connection.baseUrl}/control/unmerge`, {
    method: "POST",
    headers: authHeaders(connection),
    body: JSON.stringify({ from, to }),
  });
  if (!response.ok) throw new Error(await errorMessage(response));
}

/// Designate (or release) a `same_as` class's primary stub — the id class-level facts and reads resolve
/// through (spec §Cross-platform identity). Passing `designated` `true` pins the stub over the
/// earliest-ULID default, so a canonical handle wins over a throwaway that merely predates it; `false`
/// releases the pin. The resulting `ClassPrimaryDesignated` arrives through the live tail, so the derived
/// proposal list re-derives on the next poll. Throws with the agent's reason on failure (a `404` when the
/// id names no memory).
export async function designatePrimary(
  connection: LiveConnection,
  memory: MemoryId,
  designated: boolean,
): Promise<void> {
  const response = await fetch(`${connection.baseUrl}/control/designate-primary`, {
    method: "POST",
    headers: authHeaders(connection),
    body: JSON.stringify({ memory, designated }),
  });
  if (!response.ok) throw new Error(await errorMessage(response));
}

/// Write a graph snapshot now — the operator's take-one-before-an-experiment trigger. Returns the file
/// written, or `null` when the graph was already checkpointed at its current head (nothing to do).
/// Throws when snapshotting is disabled (the server answers `409`).
export async function snapshotNow(connection: LiveConnection): Promise<string | null> {
  const response = await fetch(`${connection.baseUrl}/control/snapshot`, {
    method: "POST",
    headers: authHeaders(connection),
  });
  if (!response.ok) throw new Error(await errorMessage(response));
  const body = (await response.json()) as { snapshot: string | null };
  return body.snapshot;
}
