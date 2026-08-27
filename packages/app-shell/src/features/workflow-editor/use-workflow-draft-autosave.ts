import { useCallback, useEffect, useRef, useState } from "react";

/** Quiet period after the last persistable edit before the draft is written. */
export const WORKFLOW_DRAFT_AUTOSAVE_MS = 1_000;

export type WorkflowDraftSaveStatus = "clean" | "dirty" | "saving" | "error";

type SaveAttemptResult = "saved" | "stale" | "skipped" | "failed" | "noop";

type UseWorkflowDraftAutosaveOptions = {
  /** When false, edits are ignored and any pending timer is cleared (e.g. version preview). */
  enabled: boolean;
  debounceMs?: number;
  /**
   * Persists the live draft. Must return whether the write still matches the
   * generation that started the save so overlapping edits can reschedule.
   */
  save: () => Promise<"saved" | "stale" | "skipped" | "failed">;
};

type UseWorkflowDraftAutosaveResult = {
  status: WorkflowDraftSaveStatus;
  /** Records a persistable local edit and (re)starts the debounce timer. */
  markDirty: () => void;
  /**
   * Cancels the timer and writes until the draft is clean or a write fails.
   * Returns false when the write failed/skipped so callers can abort navigation.
   */
  flush: (options?: { force?: boolean }) => Promise<boolean>;
  /** Drops pending dirty state without writing (e.g. deleting the open workflow). */
  cancel: () => void;
};

/**
 * Coalesces draft edits into debounced backend writes while serializing overlapping saves.
 * The parent owns the actual persist call so it can read the latest React Flow snapshot.
 *
 * Dirty state survives temporary disable (version preview) and is flushed on unmount so a
 * route change does not drop the last quiet-period edits.
 */
export function useWorkflowDraftAutosave({
  enabled,
  debounceMs = WORKFLOW_DRAFT_AUTOSAVE_MS,
  save,
}: UseWorkflowDraftAutosaveOptions): UseWorkflowDraftAutosaveResult {
  const [status, setStatus] = useState<WorkflowDraftSaveStatus>("clean");
  const enabledRef = useRef(enabled);
  const saveRef = useRef(save);
  const statusRef = useRef<WorkflowDraftSaveStatus>("clean");
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const generationRef = useRef(0);
  const dirtyRef = useRef(false);
  const inFlightRef = useRef<Promise<SaveAttemptResult> | null>(null);
  const runSaveRef = useRef<() => Promise<SaveAttemptResult>>(
    async () => "noop",
  );
  const scheduleRef = useRef<() => void>(() => undefined);

  const clearTimer = useCallback(() => {
    if (timerRef.current !== null) {
      clearTimeout(timerRef.current);
      timerRef.current = null;
    }
  }, []);

  const setSaveStatus = useCallback((next: WorkflowDraftSaveStatus) => {
    statusRef.current = next;
    setStatus((current) => (current === next ? current : next));
  }, []);

  const schedule = useCallback(() => {
    clearTimer();
    if (!enabledRef.current || !dirtyRef.current) {
      return;
    }
    timerRef.current = setTimeout(() => {
      timerRef.current = null;
      void runSaveRef.current();
    }, debounceMs);
  }, [clearTimer, debounceMs]);

  const runSave = useCallback(async (): Promise<SaveAttemptResult> => {
    if (inFlightRef.current !== null) {
      // Join the in-flight write; that attempt already reschedules on stale.
      return inFlightRef.current;
    }
    if (!dirtyRef.current || !enabledRef.current) {
      return "noop";
    }

    const startedGeneration = generationRef.current;
    // Clear dirty only for this attempt; skipped/failed restores it so data is not
    // marked clean after a no-op or error.
    dirtyRef.current = false;
    setSaveStatus("saving");

    const promise = (async (): Promise<SaveAttemptResult> => {
      const result = await saveRef.current();
      if (result === "failed") {
        dirtyRef.current = true;
        setSaveStatus("error");
        return result;
      }
      if (result === "skipped") {
        // Nothing was written (preview/teardown). Keep dirty so a later flush or
        // re-enable can still persist the pending local edits.
        dirtyRef.current = true;
        setSaveStatus("dirty");
        return result;
      }
      if (generationRef.current !== startedGeneration || result === "stale") {
        dirtyRef.current = true;
        setSaveStatus("dirty");
        scheduleRef.current();
        return "stale";
      }
      setSaveStatus("clean");
      return "saved";
    })();

    inFlightRef.current = promise;
    try {
      return await promise;
    } finally {
      if (inFlightRef.current === promise) {
        inFlightRef.current = null;
      }
    }
  }, [setSaveStatus]);

  const markDirty = useCallback(() => {
    if (!enabledRef.current) {
      return;
    }
    generationRef.current += 1;
    dirtyRef.current = true;
    // Avoid re-rendering on every drag frame once status is already dirty.
    setSaveStatus("dirty");
    schedule();
  }, [schedule, setSaveStatus]);

  const flush = useCallback(
    async (options?: { force?: boolean }): Promise<boolean> => {
      clearTimer();
      if (inFlightRef.current !== null) {
        await inFlightRef.current;
      }
      // Manual save / workflow switch must write even when only the React Flow
      // viewport changed, because pan/zoom never marks the draft dirty.
      if (options?.force === true && enabledRef.current) {
        dirtyRef.current = true;
        generationRef.current += 1;
      }
      // Keep writing until pending edits are fully drained. A single stale attempt
      // must not report success or navigation would drop newer local edits.
      while (enabledRef.current && dirtyRef.current) {
        clearTimer();
        const result = await runSave();
        if (result === "failed" || result === "skipped") {
          return false;
        }
      }
      return !dirtyRef.current;
    },
    [clearTimer, runSave],
  );

  const cancel = useCallback(() => {
    clearTimer();
    dirtyRef.current = false;
    generationRef.current += 1;
    setSaveStatus("clean");
  }, [clearTimer, setSaveStatus]);

  // Publish the latest closures after render so timers/unmount always see current values.
  useEffect(() => {
    enabledRef.current = enabled;
    saveRef.current = save;
    scheduleRef.current = schedule;
    runSaveRef.current = runSave;
  });

  // Preview disables autosave; keep dirty and resume the debounce when editing returns.
  useEffect(() => {
    if (!enabled) {
      clearTimer();
      return;
    }
    if (dirtyRef.current) {
      schedule();
    }
  }, [clearTimer, enabled, schedule]);

  // Best-effort persist on unmount so leaving the editor does not drop
  // edits still inside the debounce window. Hard tab closes remain best-effort.
  useEffect(
    () => () => {
      clearTimer();
      if (!dirtyRef.current) {
        return;
      }
      // Swallow rejection: unmount must not leave an unhandled rejection on stderr
      // after the suite already passed (run-with-clean-stderr treats that as fail).
      void saveRef.current().catch(() => undefined);
    },
    [clearTimer],
  );

  return { status, markDirty, flush, cancel };
}
