/**
 * Groups items by key while reusing prior bucket array identities when the
 * bucket's item references are unchanged. Memoized tree nodes can then keep
 * stable props across unrelated list patches (rename/delete of another branch).
 */
export function groupByStable<T>(
  items: readonly T[],
  keyOf: (item: T) => string,
  previous?: ReadonlyMap<string, readonly T[]>,
): Map<string, readonly T[]> {
  const drafted = new Map<string, T[]>();
  for (const item of items) {
    const key = keyOf(item);
    const bucket = drafted.get(key);
    if (bucket) bucket.push(item);
    else drafted.set(key, [item]);
  }

  if (previous === undefined) return drafted;

  const next = new Map<string, readonly T[]>();
  for (const [key, bucket] of drafted) {
    const prior = previous.get(key);
    if (
      prior !== undefined &&
      prior.length === bucket.length &&
      prior.every((item, index) => item === bucket[index])
    ) {
      next.set(key, prior);
    } else {
      next.set(key, bucket);
    }
  }
  return next;
}
