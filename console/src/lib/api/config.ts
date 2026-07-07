import type { LiveConnection } from "./live.ts";
import { authHeaders, errorMessage } from "./http.ts";

/// The environmental config as a read-only value tree — storage paths, endpoints, the bind address,
/// snapshots, and MCP servers. The server redacts secrets before it reaches here (API keys arrive as
/// counts, MCP env as its variable names), so there is nothing sensitive to display.
export type ConfigValue = string | number | boolean | null | ConfigValue[] | ConfigTree;
export interface ConfigTree {
  [key: string]: ConfigValue;
}

/// The environmental config this instance booted from (`GET /control/config`), read-only.
export async function getConfig(connection: LiveConnection): Promise<ConfigTree> {
  const response = await fetch(`${connection.baseUrl}/control/config`, {
    headers: authHeaders(connection),
  });
  if (!response.ok) throw new Error(await errorMessage(response));
  return (await response.json()) as ConfigTree;
}
