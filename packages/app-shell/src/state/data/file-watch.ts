import type { WorkspaceFileEventBatch } from "@ora/contracts";

const INITIAL_RECONNECT_DELAY_MS = 500;
const MAX_RECONNECT_DELAY_MS = 10_000;

interface WorkspaceWatchLoopOptions {
  signal: AbortSignal;
  openStream: (signal: AbortSignal) => AsyncIterable<WorkspaceFileEventBatch>;
  onBatch: (batch: WorkspaceFileEventBatch) => Promise<void> | void;
  wait?: (delayMs: number, signal: AbortSignal) => Promise<void>;
}

/** Keeps a workspace event stream alive until its owning component aborts the operation. */
export async function watchWorkspaceContinuously({
  signal,
  openStream,
  onBatch,
  wait = waitForReconnect,
}: WorkspaceWatchLoopOptions): Promise<void> {
  let reconnectAttempt = 0;
  while (!signal.aborted) {
    try {
      for await (const batch of openStream(signal)) {
        if (signal.aborted) return;
        reconnectAttempt = 0;
        await onBatch(batch);
      }
    } catch (error) {
      if (signal.aborted || isAbortError(error)) return;
    }
    if (signal.aborted) return;

    await wait(workspaceWatchReconnectDelay(reconnectAttempt), signal);
    reconnectAttempt += 1;
  }
}

/** Applies bounded exponential backoff so a down backend cannot create a reconnect storm. */
export function workspaceWatchReconnectDelay(attempt: number): number {
  return Math.min(
    INITIAL_RECONNECT_DELAY_MS * 2 ** Math.max(0, attempt),
    MAX_RECONNECT_DELAY_MS,
  );
}

/** Waits for the next connection attempt while allowing unmount to cancel immediately. */
function waitForReconnect(delayMs: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    if (signal.aborted) {
      resolve();
      return;
    }
    const finish = () => {
      clearTimeout(timer);
      signal.removeEventListener("abort", finish);
      resolve();
    };
    const timer = setTimeout(finish, delayMs);
    signal.addEventListener("abort", finish, { once: true });
  });
}

/** Recognizes transport aborts across browser implementations without hiding other failures. */
function isAbortError(error: unknown): boolean {
  return error instanceof DOMException && error.name === "AbortError";
}
