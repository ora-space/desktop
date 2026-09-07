/** Allocates deterministic fixture ids without touching global state. */
export function nextId(prefix: string, count: number): string {
  return `${prefix}${count + 1}`;
}

/** Produces a millisecond-precision timestamp matching the contract's bigint wire type. */
export function nextTimestamp(): bigint {
  return BigInt(Date.now());
}
