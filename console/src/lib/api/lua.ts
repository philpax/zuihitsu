import type { ApiEntry } from "@zuihitsu/wire/types/ApiEntry.ts";
import type { ApiType } from "@zuihitsu/wire/types/ApiType.ts";
import type { LiveConnection } from "./live.ts";
import { authHeaders, errorMessage } from "./http.ts";

/// The structured Lua API catalogue the agent projects into its system prompt, served by
/// `GET /control/lua-api` for the console to render as a reference guide. The shapes are generated
/// from the Rust `zuihitsu_frontend_types::api` types (`ApiGate` marks a call gated on an opt-in);
/// re-exported here so the reference components import the whole catalogue from one place.
export type { ApiType } from "@zuihitsu/wire/types/ApiType.ts";
export type { ApiParam } from "@zuihitsu/wire/types/ApiParam.ts";
export type { ApiGate } from "@zuihitsu/wire/types/ApiGate.ts";
export type { ApiEntry } from "@zuihitsu/wire/types/ApiEntry.ts";

/// The result of a Lua console run: the rendered value, or the error/abort that ended it. Exactly
/// one is non-null (mirrors the Rust `LuaConsoleOutcome`).
export interface LuaOutcome {
  result: string | null;
  error: string | null;
}

/// Run an operator Lua block in the agent's no-commit sandbox. `allowMcp` opts the block into real
/// MCP calls and `allowWeb` into `web.markdown`; both are off by default, since each performs external
/// I/O even though memory writes are discarded. Throws on an infrastructure failure; a *script* error
/// returns as `outcome.error`.
export async function runLua(
  connection: LiveConnection,
  script: string,
  allowMcp: boolean,
  allowWeb: boolean,
): Promise<LuaOutcome> {
  const response = await fetch(`${connection.baseUrl}/control/lua`, {
    method: "POST",
    headers: authHeaders(connection),
    body: JSON.stringify({ script, allow_mcp: allowMcp, allow_web: allowWeb }),
  });
  if (!response.ok) throw new Error(await errorMessage(response));
  return (await response.json()) as LuaOutcome;
}

/// The Lua API catalogue, for the reference guide.
export async function luaApi(connection: LiveConnection): Promise<ApiEntry[]> {
  const response = await fetch(`${connection.baseUrl}/control/lua-api`, {
    headers: authHeaders(connection),
  });
  if (!response.ok) throw new Error(await errorMessage(response));
  return (await response.json()) as ApiEntry[];
}

/// A type rendered the way the agent's own API doc renders it — "memory handle", "string list",
/// "{…} or nil" — so the reference reads the same as the prompt the agent sees.
export function formatType(type: ApiType): string {
  if (typeof type === "string") {
    return {
      String: "string",
      Integer: "integer",
      Number: "number",
      Boolean: "boolean",
      Handle: "memory handle",
      Entry: "entry",
      Nil: "nil",
      Any: "any",
    }[type];
  }
  if ("Object" in type) return "{…}";
  if ("List" in type) return `${formatType(type.List)} list`;
  if ("Enum" in type) return type.Enum.map((value) => `"${value}"`).join(" | ");
  return `${formatType(type.Optional)} or nil`;
}
