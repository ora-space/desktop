import type {
  ContractsClient,
  ScheduledTask,
  ScheduledTaskEvent,
  ScheduledTaskHistoryEntry,
} from "@ora/contracts";
import type { QueryClient } from "@tanstack/react-query";
import { appI18n } from "../../i18n/i18n-instance";
import type { PlatformAdapter } from "../../platform";
import { scheduledTaskKeys } from "../hooks/use-scheduled-tasks";
import { useUiStore } from "../stores/ui-store";
import { notifyFailedScheduledFact } from "./scheduled-task-events";

const SEEN_STORAGE_KEY = "ora.scheduledTaskFactsSeenAt";
const SEEN_FACTS_STORAGE_KEY = "ora.scheduledTaskFactsSeen";
const SEEN_FACTS_INITIALIZED_STORAGE_KEY =
  "ora.scheduledTaskFactsSeenInitialized";
const MAX_SEEN_FACTS = 1_000;

/** Reads the timestamp up to which persisted facts were already surfaced to the user. */
function readSeenBaseline(): number {
  try {
    const raw = window.localStorage.getItem(SEEN_STORAGE_KEY);
    if (raw === null) return 0;
    const value = Number(raw);
    return Number.isFinite(value) ? value : 0;
  } catch {
    return 0;
  }
}

/** Persists the surfaced timestamp; a lost write can only repeat a notification. */
function writeSeenBaseline(at: number): void {
  try {
    window.localStorage.setItem(SEEN_STORAGE_KEY, String(at));
  } catch {
    // Best effort: surfacing must not depend on storage availability.
  }
}

/** Advances the durable notification baseline without allowing a concurrent path to regress it. */
function advanceSeenBaseline(at: number): void {
  if (at > readSeenBaseline()) writeSeenBaseline(at);
}

/** Stores exact facts so one task's live event cannot suppress another task's history. */
function readSeenFacts(): Record<string, number> {
  try {
    const raw = window.localStorage.getItem(SEEN_FACTS_STORAGE_KEY);
    if (raw === null) return {};
    const value: unknown = JSON.parse(raw);
    if (value === null || typeof value !== "object" || Array.isArray(value)) {
      return {};
    }
    return Object.fromEntries(
      Object.entries(value).filter(
        ([, seenAt]) => typeof seenAt === "number" && Number.isFinite(seenAt),
      ),
    );
  } catch {
    return {};
  }
}

function writeSeenFacts(facts: Record<string, number>): void {
  try {
    const retained = Object.entries(facts)
      .sort(([, left], [, right]) => right - left)
      .slice(0, MAX_SEEN_FACTS);
    window.localStorage.setItem(
      SEEN_FACTS_STORAGE_KEY,
      JSON.stringify(Object.fromEntries(retained)),
    );
  } catch {
    // Best effort: surfacing must not depend on storage availability.
  }
}

function hasSeenFact(key: string): boolean {
  return Object.hasOwn(readSeenFacts(), key);
}

function markFactSeen(key: string, seenAt: number): void {
  writeSeenFacts({ ...readSeenFacts(), [key]: seenAt });
}

function seenFactsInitialized(): boolean {
  try {
    return (
      window.localStorage.getItem(SEEN_FACTS_INITIALIZED_STORAGE_KEY) === "1"
    );
  } catch {
    return false;
  }
}

function markSeenFactsInitialized(): void {
  try {
    window.localStorage.setItem(SEEN_FACTS_INITIALIZED_STORAGE_KEY, "1");
  } catch {
    // The legacy baseline remains a safe fallback when storage is unavailable.
  }
}

function asMillis(value: bigint): number {
  return Number(value);
}

/** Reports whether the entry is a durable failure or missed fact the user must hear about. */
function surfaceableFactAt(entry: ScheduledTaskHistoryEntry): number | null {
  if (entry.kind === "missed") return asMillis(entry.observedAt);
  if (entry.failureReason !== null) return asMillis(entry.updatedAt);
  return null;
}

function factKey(
  taskId: string,
  entry: ScheduledTaskHistoryEntry,
): string | null {
  if (entry.kind === "missed") {
    return `missed:${taskId}:${entry.scheduledAt}`;
  }
  return entry.failureReason === null ? null : `run:${taskId}:${entry.id}`;
}

function liveFact(
  entries: ScheduledTaskHistoryEntry[],
  event: Extract<
    ScheduledTaskEvent,
    { kind: "run_finished" | "occurrence_missed" }
  >,
): ScheduledTaskHistoryEntry | null {
  if (event.kind === "occurrence_missed") {
    return (
      entries.find(
        (entry) =>
          entry.kind === "missed" && entry.scheduledAt === event.scheduledAt,
      ) ?? null
    );
  }
  return (
    entries.find(
      (entry): entry is Extract<ScheduledTaskHistoryEntry, { kind: "run" }> =>
        entry.kind === "run" && entry.id === event.runId,
    ) ?? null
  );
}

/**
 * Serializes every baseline-advancing pass.
 *
 * The startup surface and the live-event dedupe both read, decide, and advance the same
 * durable baseline. Running them concurrently would let a live event advance the baseline
 * over older facts the in-flight surface has not shown yet, silently dropping them (the
 * dedupe must only prevent repeats, never lose facts). One shared chain keeps each pass's
 * reads and baseline write contiguous; live toasts may wait one surface pass, which is
 * bounded by the per-task history reads.
 */
let baselineChain: Promise<void> = Promise.resolve();

function serializeBaselineTurn<T>(turn: () => Promise<T>): Promise<T> {
  const result = baselineChain.then(turn, turn);
  baselineChain = result.then(
    () => undefined,
    () => undefined,
  );
  return result;
}

/**
 * Claims one persisted fact for a live event. The caller surfaces the toast only when this
 * returns true, so a startup pass and the live stream cannot toast the same fact twice.
 */
export function markScheduledTaskHistorySeen(
  client: ContractsClient,
  queryClient: QueryClient,
  event: Extract<
    ScheduledTaskEvent,
    { kind: "run_finished" | "occurrence_missed" }
  >,
): Promise<boolean> {
  return serializeBaselineTurn(async () => {
    try {
      const entries = await client.scheduledTask
        .listHistory({ taskId: event.taskId })
        .then((response) => response.entries);
      queryClient.setQueryData(
        scheduledTaskKeys.history(event.taskId),
        entries,
      );
      const fact = liveFact(entries, event);
      if (fact === null) return true;
      const key = factKey(event.taskId, fact);
      if (key === null) return true;
      if (hasSeenFact(key)) return false;
      markFactSeen(key, surfaceableFactAt(fact) ?? Date.now());
      return true;
    } catch {
      // The event remains actionable even when its history refresh is temporarily unavailable:
      // the toast fires, the fact is not marked seen, and a later startup pass may surface it
      // again. This is an accepted trade-off — a duplicate notification is preferred over a
      // silently dropped one (ora-space/circus#333).
      return true;
    }
  });
}

let startupSurfaceInFlight: Promise<void> | null = null;
let startupSurfaceQueuedPromise: Promise<void> | null = null;

/**
 * Surfaces persisted Scheduled Task facts that were recorded while the shell could not
 * receive application events: startup-recovered missed occurrences and interrupted failed
 * runs. Exact fact keys prevent duplicate notifications; the first run on an install adopts
 * the existing history as already seen instead of replaying it as a notification storm.
 *
 * A request made while a pass is in flight queues exactly one follow-up pass instead of
 * reusing the in-flight promise: facts persisted during the first pass (for example while
 * a reconnect window had no subscriber) would otherwise wait for the next restart. The
 * returned promise resolves when the pass serving this call has finished, so awaiting
 * callers wait out a queued follow-up too.
 */
export function surfaceStartupScheduledFacts(
  client: ContractsClient,
  queryClient: QueryClient,
  platform: PlatformAdapter | null,
): Promise<void> {
  if (startupSurfaceInFlight !== null) {
    if (startupSurfaceQueuedPromise === null) {
      startupSurfaceQueuedPromise = startupSurfaceInFlight.then(() =>
        surfaceStartupScheduledFacts(client, queryClient, platform),
      );
    }
    return startupSurfaceQueuedPromise;
  }
  const surface = serializeBaselineTurn(() =>
    surfaceStartupScheduledFactsOnce(client, queryClient, platform),
  );
  startupSurfaceInFlight = surface;
  void surface.finally(() => {
    if (startupSurfaceInFlight === surface) startupSurfaceInFlight = null;
    startupSurfaceQueuedPromise = null;
  });
  return surface;
}

async function surfaceStartupScheduledFactsOnce(
  client: ContractsClient,
  queryClient: QueryClient,
  platform: PlatformAdapter | null,
): Promise<void> {
  const t = appI18n.t.bind(appI18n);
  // The very first run on an install adopts the existing history as already seen instead of
  // replaying every historical failure as a notification storm. The pass start time bounds the
  // adoption so that a partial history read (some tasks failing) does not silently absorb new
  // facts that were persisted after the pass began (edge case from ora-space/circus#333).
  const migrateOnly = !seenFactsInitialized();
  const passStartedAt = Date.now();
  const legacyBaseline = migrateOnly ? readSeenBaseline() : 0;
  let newest = legacyBaseline;
  let historiesComplete = true;
  const seen = readSeenFacts();
  let seenDirty = false;
  const hasSeen = (key: string): boolean => Object.hasOwn(seen, key);
  const markSeen = (key: string, seenAt: number): void => {
    seen[key] = seenAt;
    seenDirty = true;
  };

  let tasks: ScheduledTask[];
  try {
    tasks = await queryClient.fetchQuery({
      queryKey: scheduledTaskKeys.all,
      queryFn: () => client.scheduledTask.list({}).then((r) => r.tasks),
    });
  } catch {
    return;
  }

  for (const task of tasks) {
    let entries: ScheduledTaskHistoryEntry[];
    try {
      entries = await queryClient.fetchQuery({
        queryKey: scheduledTaskKeys.history(task.id),
        queryFn: () =>
          client.scheduledTask
            .listHistory({ taskId: task.id })
            .then((r) => r.entries),
      });
    } catch {
      historiesComplete = false;
      continue;
    }
    const baseline = readSeenBaseline();
    for (const entry of entries) {
      const factAt = surfaceableFactAt(entry);
      const key = factKey(task.id, entry);
      if (factAt === null || key === null || hasSeen(key)) continue;
      // On the first run (migrate-only), adopt all facts. On subsequent passes, only adopt
      // facts at or before the legacy baseline, and never silently absorb facts that appeared
      // after this pass started — those will be surfaced by the next pass or by the live
      // event stream instead.
      if (migrateOnly || factAt <= baseline) {
        newest = Math.max(newest, factAt);
        markSeen(key, factAt);
        continue;
      }
      if (factAt > passStartedAt) {
        // A fact that appeared after this pass started was persisted by a live path that
        // already surfaced it (or will surface it). Do not silently absorb it here.
        continue;
      }
      newest = Math.max(newest, factAt);
      const title =
        entry.kind === "missed"
          ? t("scheduledTask.notificationMissedTitle")
          : t("scheduledTask.notificationFailedTitle");
      const reason =
        entry.kind === "missed"
          ? t(`scheduledTask.historyReason.${entry.reason}`)
          : t(`scheduledTask.failureReason.${entry.failureReason}`);
      await notifyFailedScheduledFact(
        platform,
        title,
        `${task.name} · ${reason}`,
        () =>
          useUiStore.getState().openScheduledTaskHistory({ taskId: task.id }),
      );
      markSeen(key, factAt);
    }
  }

  if (seenDirty) writeSeenFacts(seen);
  if (newest > legacyBaseline) advanceSeenBaseline(newest);
  if (historiesComplete) markSeenFactsInitialized();
}
