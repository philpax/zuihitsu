import type { Settings } from "../types/Settings.ts";
import type { LiveConnection } from "./live.ts";

export type { Settings };

/// The agent's current behavioral settings (the latest `ConfigSet` snapshot).
export async function getSettings(connection: LiveConnection): Promise<Settings> {
  const response = await fetch(`${connection.baseUrl}/control/settings`, {
    headers: authHeaders(connection),
  });
  if (!response.ok) throw new Error(await errorMessage(response));
  return (await response.json()) as Settings;
}

/// Replace the behavioral settings — logged as an operator `ConfigSet`, taking effect on the next
/// read (a read-modify-write of the whole snapshot).
export async function putSettings(connection: LiveConnection, settings: Settings): Promise<void> {
  const response = await fetch(`${connection.baseUrl}/control/settings`, {
    method: "PUT",
    headers: authHeaders(connection),
    body: JSON.stringify(settings),
  });
  if (!response.ok) throw new Error(await errorMessage(response));
}

function authHeaders(connection: LiveConnection): HeadersInit {
  const headers: Record<string, string> = { "content-type": "application/json" };
  if (connection.key) headers.Authorization = `Bearer ${connection.key}`;
  return headers;
}

async function errorMessage(response: Response): Promise<string> {
  try {
    const body = (await response.json()) as { error?: string };
    if (body.error) return body.error;
  } catch {
    /* fall through to the status line */
  }
  return `the agent answered ${response.status} ${response.statusText}`;
}
