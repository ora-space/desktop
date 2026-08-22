import { useMemo, useRef } from "react";
import { groupByStable } from "./group-by-stable";

/**
 * Groups `items` with bucket-identity reuse across updates.
 *
 * The previous map is read/written inside `useMemo` (not during the broad
 * render body) so unchanged project/task buckets keep stable array identities
 * for memoized tree nodes. Callers must pass a stable `items` reference
 * (e.g. from `useMemo`) — inline filters would defeat both memo layers.
 */
export function useStableGroupBy<T>(
  items: readonly T[],
  keyOf: (item: T) => string,
): Map<string, readonly T[]> {
  const previousRef = useRef<Map<string, readonly T[]>>(new Map());
  return useMemo(() => {
    // react-hooks/refs: intentional cache across memoized recalculations only.
    // eslint-disable-next-line react-hooks/refs -- previous map is a memo input, not render state
    const previous = previousRef.current;
    const next = groupByStable(items, keyOf, previous);
    // eslint-disable-next-line react-hooks/refs -- store for the next items identity change
    previousRef.current = next;
    return next;
  }, [items, keyOf]);
}
