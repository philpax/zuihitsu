import type { ConversationLocator } from "@zuihitsu/wire/types/ConversationLocator.ts";
import type { PlatformResponse } from "@zuihitsu/wire/types/PlatformResponse.ts";
import type { LiveConnection } from "./live.ts";
import { authHeaders, errorMessage } from "./http.ts";

// The platform key the console uses for its own conversations — the spec's "direct interface"
// platform client. A room the console starts is addressed `(direct, <name>)`; the console can also
// drop into a room that arrived from another platform by reusing that room's locator. Sourced from
// Rust (`ids::DIRECT_PLATFORM`) so the value stays identical to the one identity resolution keys its
// operator-authority merge on; re-exported here for consumers already reaching for the api module.
export { DIRECT_PLATFORM } from "@zuihitsu/wire/types/constants.ts";

/// One participant turn the console delivers as a platform client: the room, who is speaking (a
/// platform handle the server resolves to a `person/*` stub), what they said, and who is present.
export interface OutboundMessage {
  locator: ConversationLocator;
  sender: string;
  text: string;
  present: string[];
}

/// Deliver a participant message and run the agent's response cycle — the console acting as a
/// platform client (spec §Clients → platform clients), holding only platform authority. The reply
/// and both turns arrive through the live tail; the returned response carries the outcome (which
/// matters mostly for `"Deferred"`, saying the message was delivered and recorded but the agent's
/// model was unreachable — the agent catches up on its next turn) and the participant's turn id.
/// Throws with the agent's reason on failure (e.g. no model configured).
export async function sendMessage(
  connection: LiveConnection,
  message: OutboundMessage,
): Promise<PlatformResponse> {
  const response = await fetch(`${connection.baseUrl}/platform/message`, {
    method: "POST",
    headers: authHeaders(connection),
    body: JSON.stringify(message),
  });
  if (!response.ok) throw new Error(await errorMessage(response));
  return (await response.json()) as PlatformResponse;
}
