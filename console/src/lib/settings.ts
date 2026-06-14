import type { Settings } from "../types/Settings.ts";
import type { LiveConnection } from "./live.ts";
import { authHeaders, errorMessage } from "./http.ts";

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
