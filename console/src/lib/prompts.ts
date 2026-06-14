import type { Event } from "../types/Event.ts";
import type { PromptTemplateName } from "../types/PromptTemplateName.ts";
import type { LiveConnection } from "./live.ts";
import { authHeaders, errorMessage } from "./http.ts";

/// One prompt template at its current version — the highest version registered under a name (spec
/// §Initialization → prompt templates). The agent reads the system-prompt scaffold and the framing
/// templates from these.
export interface PromptTemplate {
  name: PromptTemplateName;
  version: number;
  body: string;
}

/// The current prompt templates up to the cursor: the highest-versioned `PromptTemplateRegistered`
/// per name, in registration order. Templates are read from the log, never materialized, so the
/// console derives them the same way the agent does.
export function deriveTemplates(events: Event[], cursor: number): PromptTemplate[] {
  const latest = new Map<PromptTemplateName, PromptTemplate>();
  for (const event of events) {
    if (event.seq > cursor) continue;
    const payload = event.payload;
    if (payload.type !== "PromptTemplateRegistered") continue;
    const current = latest.get(payload.name);
    if (!current || payload.version >= current.version) {
      latest.set(payload.name, {
        name: payload.name,
        version: payload.version,
        body: payload.body,
      });
    }
  }
  return [...latest.values()];
}

/// Register a new version of a prompt template — the operator edit. The new version replaces the
/// template from the next read on, and arrives back through the live tail like any other event.
export async function registerPrompt(
  connection: LiveConnection,
  name: PromptTemplateName,
  body: string,
): Promise<void> {
  const response = await fetch(`${connection.baseUrl}/control/prompt`, {
    method: "POST",
    headers: authHeaders(connection),
    body: JSON.stringify({ name, body }),
  });
  if (!response.ok) throw new Error(await errorMessage(response));
}
