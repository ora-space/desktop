import { useSyncExternalStore } from "react";

const listeners = new Set<() => void>();
let timer: ReturnType<typeof setInterval> | null = null;
let nowMs = Date.now();

function publish(): void {
  nowMs = Date.now();
  for (const listener of listeners) listener();
}

function reconcileTimer(): void {
  const shouldRun =
    listeners.size > 0 && document.visibilityState === "visible";
  if (shouldRun && timer === null) timer = setInterval(publish, 1_000);
  if (!shouldRun && timer !== null) {
    clearInterval(timer);
    timer = null;
  }
}

function onVisibilityChange(): void {
  if (document.visibilityState === "visible") publish();
  reconcileTimer();
}

/** Subscribes one active elapsed label to the renderer-wide one-second clock. */
function subscribe(listener: () => void): () => void {
  const wasEmpty = listeners.size === 0;
  listeners.add(listener);
  if (wasEmpty) {
    // A newly visible label must catch up immediately after navigation or background
    // throttling instead of waiting for the first interval tick.
    nowMs = Date.now();
    document.addEventListener("visibilitychange", onVisibilityChange);
  }
  reconcileTimer();
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0)
      document.removeEventListener("visibilitychange", onVisibilityChange);
    reconcileTimer();
  };
}

function snapshot(): number {
  return nowMs;
}

const subscribeNever = () => () => undefined;

/** Returns live elapsed milliseconds, or a fixed terminal duration without subscribing. */
export function useElapsedDuration(
  startedAt: number | undefined,
  durationMs: number | undefined,
): number | undefined {
  const active = startedAt !== undefined && durationMs === undefined;
  const now = useSyncExternalStore(
    active ? subscribe : subscribeNever,
    snapshot,
    snapshot,
  );
  if (durationMs !== undefined) return durationMs;
  return startedAt === undefined ? undefined : Math.max(0, now - startedAt);
}
