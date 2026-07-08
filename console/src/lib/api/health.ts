import { useEffect, useState } from "react";

import type { BackendHealth } from "../../types/BackendHealth.ts";
import type { GenesisStatus } from "./operator.ts";
import type { LiveConnection } from "./live.ts";
import { authHeaders, errorMessage } from "./http.ts";

/// The serving health `GET /control/health` reports — mirrors the Rust `Health` response: whether
/// the agent exists, and the model transport's circuit. `model` is `null` when no model endpoint is
/// configured (the conversing endpoints answer 503, which is its own signal).
export interface InstanceHealth {
  genesis: GenesisStatus;
  model: BackendHealth | null;
}

export async function instanceHealth(connection: LiveConnection): Promise<InstanceHealth> {
  const response = await fetch(`${connection.baseUrl}/control/health`, {
    headers: authHeaders(connection),
  });
  if (!response.ok) throw new Error(await errorMessage(response));
  return (await response.json()) as InstanceHealth;
}

/// Whether the model transport is degraded — the circuit is open (or probing half-open), or the
/// last calls failed transiently and the wrapper is retrying. The degraded-backend banner shows
/// while this holds and disappears on recovery (a closed circuit with a clean failure count).
export function isDegraded(health: BackendHealth | null): health is BackendHealth {
  return health !== null && (health.circuit !== "closed" || health.consecutive_failures > 0);
}

/// How often the console re-reads the transport health. Frequent enough that the banner appears
/// within a few seconds of the circuit opening and clears promptly on recovery; far too slow to
/// matter as load (one small JSON read).
const HEALTH_POLL_MS = 5_000;

/// Poll the model transport's health while mounted. Deliberately quiet about its own failures: an
/// unreachable console connection yields `null` (no banner) rather than an error — the header's
/// connection badge already covers "cannot reach the agent"; this hook is only about the agent's
/// own model backend.
export function useBackendHealth(connection: LiveConnection): BackendHealth | null {
  const [health, setHealth] = useState<BackendHealth | null>(null);
  useEffect(() => {
    let cancelled = false;
    const poll = () => {
      instanceHealth(connection).then(
        (value) => !cancelled && setHealth(value.model),
        () => !cancelled && setHealth(null),
      );
    };
    poll();
    const timer = setInterval(poll, HEALTH_POLL_MS);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [connection]);
  return health;
}
