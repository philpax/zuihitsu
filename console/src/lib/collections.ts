/// Bucket `items` into groups keyed by `keyOf`, preserving first-seen order of both the keys and the
/// items within each group. The caller supplies the key (a namespace prefix, a category) and may sort
/// the returned pairs; this owns only the grouping, which is otherwise re-hand-rolled per view.
export function groupBy<T>(items: T[], keyOf: (item: T) => string): Array<[string, T[]]> {
  const groups = new Map<string, T[]>();
  for (const item of items) {
    const key = keyOf(item);
    const bucket = groups.get(key);
    if (bucket) bucket.push(item);
    else groups.set(key, [item]);
  }
  return [...groups.entries()];
}
