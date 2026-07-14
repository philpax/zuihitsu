/// Build a `conversationId → contextMemoryName` map from conversations, so `ConversationRef`
/// links can resolve the room name. The context memory name is what `nameById` holds for it
/// (e.g. `context/discord:book-club`), so the caller can then resolve it to a display name.
export function conversationNameById(
  conversations: { id: string; context_name: string | null }[],
): Map<string, string> {
  const map = new Map<string, string>();
  for (const conv of conversations) {
    if (conv.context_name) {
      map.set(conv.id, conv.context_name);
    }
  }
  return map;
}
