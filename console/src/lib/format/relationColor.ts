/// A stable color for a relation name, derived from a hash so the same relation reads the same
/// everywhere it surfaces — the relations graph and its legend, the linked-pairs list, the memory
/// detail pane's links, the join brief, and the event payloads. Shared so a relation is a scannable
/// visual cue across the whole console rather than a per-view choice. `same` edges (identity plumbing
/// rather than a typed relation with a registry entry) fall back to the caller's accent when one is
/// supplied — the canvas passes the palette's sage.
export function relationColor(name: string, fallback?: string): string {
  if (fallback !== undefined) {
    // The canvas calls pass the palette's sage as the fallback for `same` edges.
    if (name === "same as") return fallback;
  }
  let hash = 0;
  for (let i = 0; i < name.length; i++) {
    hash = (hash << 5) - hash + name.charCodeAt(i);
    hash |= 0;
  }
  const hue = Math.abs(hash) % 360;
  // A moderate saturation and lightness that sits comfortably on the warm paper ground.
  return `hsl(${hue}, 55%, 45%)`;
}
