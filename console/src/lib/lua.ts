import type { LiveConnection } from "./live.ts";

/// The structured Lua API catalogue the agent projects into its system prompt, served by
/// `GET /control/lua-api` for the console to render as a reference guide. These mirror the Rust
/// `api_doc` types (serde's external tagging: a unit variant is its name, a data variant an object).
export type ApiType =
  | "String"
  | "Integer"
  | "Number"
  | "Boolean"
  | "Handle"
  | "Entry"
  | "Nil"
  | "Any"
  | { Object: ApiParam[] }
  | { List: ApiType }
  | { Enum: string[] }
  | { Optional: ApiType };

export interface ApiParam {
  name: string;
  ty: ApiType;
  required: boolean;
  doc: string;
}

export interface ApiEntry {
  call: string;
  doc: string;
  params: ApiParam[];
  returns: ApiType;
}

/// The result of a Lua console run: the rendered value, or the error/abort that ended it. Exactly
/// one is non-null (mirrors the Rust `LuaConsoleOutcome`).
export interface LuaOutcome {
  result: string | null;
  error: string | null;
}

/// Run an operator Lua block in the agent's no-commit sandbox. `allowMcp` opts the block into real
/// MCP calls (off by default; an MCP call performs external I/O even though memory writes are
/// discarded). Throws on an infrastructure failure; a *script* error returns as `outcome.error`.
export async function runLua(
  connection: LiveConnection,
  script: string,
  allowMcp: boolean,
): Promise<LuaOutcome> {
  const response = await fetch(`${connection.baseUrl}/control/lua`, {
    method: "POST",
    headers: authHeaders(connection),
    body: JSON.stringify({ script, allow_mcp: allowMcp }),
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
