import { useEffect, useId, useState, useSyncExternalStore } from "react";

/**
 * Tracks which overlay primitives are currently open so a host can hide native
 * child views (plugin surfaces) that would otherwise paint above DOM overlays.
 *
 * The store is module-level and dependency-free so the ui primitives can lease
 * from it without the package learning anything about the application shell.
 */
const leases = new Map<string, string>();
const listeners = new Set<() => void>();

function notify() {
  for (const listener of listeners) listener();
}

/** Registers one open overlay; the reason is only kept for debugging. */
export function acquireOcclusionLease(id: string, reason: string): void {
  leases.set(id, reason);
  notify();
}

/** Drops a lease; a no-op for ids that were never acquired. */
export function releaseOcclusionLease(id: string): void {
  if (leases.delete(id)) notify();
}

/** Subscribes to lease count changes; returns the unsubscribe function. */
export function subscribeOcclusion(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** The number of overlays currently holding a lease. */
export function getOcclusionLeaseCount(): number {
  return leases.size;
}

/** Holds a lease for as long as the calling component is mounted with `active` true. */
export function useOcclusionLease(active: boolean, reason: string): void {
  const id = useId();
  useEffect(() => {
    if (!active) return;
    acquireOcclusionLease(id, reason);
    return () => releaseOcclusionLease(id);
  }, [active, id, reason]);
}

/** Reactive lease count for hosts that hide native views while overlays are open. */
export function useOcclusionLeaseCount(): number {
  return useSyncExternalStore(
    subscribeOcclusion,
    getOcclusionLeaseCount,
    getOcclusionLeaseCount,
  );
}

interface OpenStateProps<Args extends unknown[]> {
  open?: boolean;
  defaultOpen?: boolean;
  onOpenChange?: (open: boolean, ...args: Args) => void;
}

/**
 * Leases occlusion for a Root primitive that may be controlled or uncontrolled.
 *
 * Uncontrolled roots never expose their open state, so the hook mirrors it by
 * intercepting `onOpenChange`; the returned callback must replace the caller's.
 */
export function useRootOcclusionLease<Args extends unknown[]>(
  reason: string,
  { open, defaultOpen, onOpenChange }: OpenStateProps<Args>,
): (open: boolean, ...args: Args) => void {
  const [uncontrolledOpen, setUncontrolledOpen] = useState(
    defaultOpen ?? false,
  );
  useOcclusionLease(open ?? uncontrolledOpen, reason);
  return (next, ...args) => {
    setUncontrolledOpen(next);
    onOpenChange?.(next, ...args);
  };
}
