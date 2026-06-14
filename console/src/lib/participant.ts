import type { ConversationLocator } from "../types/ConversationLocator.ts";
import type { LiveConnection } from "./live.ts";
import { authHeaders, errorMessage } from "./http.ts";

/// The platform key the console uses for its own conversations — the spec's "direct interface"
/// platform client. A room the console starts is addressed `(direct, <name>)`; the console can also
/// drop into a room that arrived from another platform by reusing that room's locator.
export const DIRECT_PLATFORM = "direct";

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
/// and both turns arrive through the live tail. Throws with the agent's reason on failure (e.g. no
/// model configured).
export async function sendMessage(
  connection: LiveConnection,
  message: OutboundMessage,
): Promise<void> {
  const response = await fetch(`${connection.baseUrl}/platform/message`, {
    method: "POST",
    headers: authHeaders(connection),
    body: JSON.stringify(message),
  });
  if (!response.ok) throw new Error(await errorMessage(response));
}
