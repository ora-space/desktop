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

const SEEN_FACTS_STORAGE_KEY = "ora.scheduledTaskFactsSeen";
const SEEN_FACTS_INITIALIZED_STORAGE_KEY =
  "ora.scheduledTaskFactsSeenInitialized";
const MAX_SEEN_FACTS = 1_000;

/**
 * Exact per-fact deduplication ledger. Keys are deterministic fact identities
 * (`run:{runId}` or `missed:{taskId}:{scheduledAt}`); values are the persisted
 * timestamps for diagnostic purposes only — they are never compared against each
 * other or against wall clocks for suppression decisions.
 *
 * Why no timestamp baseline or per-task watermarks: timestamps from concurrent
 * transactions cannot reliably indicate commit ordering across tasks. A global or
 * per-task timestamp watermark would let a newer fact's advance silently absorb an
 * older unseen fact from a different task (the root cause of three P2 review
 * findings on ora-space/desktop#707). Key-based dedup alone avoids this: a fact is
 * suppressed if and only if its exact key has been claimed before, regardless of
 * any clock. The accepted trade-off is that the bounded map (MAX_SEEN_FACTS) may
 * evict very old entries after extended outages, causing a re-notification — this
 * duplicates rather than drops, which is the preferred failure mode.
 */
const seenFacts = readSeenFactsOnce();

/** Clears the in-session fact ledger (test isolation). */
export function resetScheduledFactSeen(): void {
  for (const key of Object.keys(seenFacts)) delete seenFacts[key];
  try {
    window.localStorage.removeItem(SEEN_FACTS_STORAGE_KEY);
  } catch {
    // Best effort.
  }
}

function readSeenFactsOnce(): Record<string, number> {
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

function persistSeenFacts(facts: Record<string, number>): void {
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
  return Object.hasOwn(seenFacts, key);
}

function markFactSeen(key: string, seenAt: number): void {
  seenFacts[key] = seenAt;
  persistSeenFacts(seenFacts);
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
    // The migration will repeat on the next pass if storage is unavailable.
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
  return entry.failureReason === null ? null : `run:${entry.id}`;
}

function eventFactKey(
  event: Extract<
    ScheduledTaskEvent,
    { kind: "run_finished" | "occurrence_missed" }
  >,
): string | null {
  if (event.kind === "occurrence_missed") {
    return `missed:${event.taskId}:${event.scheduledAt}`;
  }
  return event.failureReason === null ? null : `run:${event.runId}`;
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
 * Serializes every deduplication pass so the startup surface and the live-event
 * claim cannot interleave their read-check-mark sequences.
 */
let dedupChain: Promise<void> = Promise.resolve();

function serializeDedupTurn<T>(turn: () => Promise<T>): Promise<T> {
  const result = dedupChain.then(turn, turn);
  dedupChain = result.then(
    () => undefined,
    () => undefined,
  );
  return result;
}

/**
 * Claims one persisted fact for a live event. The caller surfaces the toast only when this
 * returns true, so a startup pass and the live stream cannot toast the same fact twice.
 * When the history refresh fails the fact still surfaces but stays unclaimed, so a later
 * startup pass may repeat it — a registered trade-off, since a duplicate beats a silent
 * loss.
 */
export function markScheduledTaskHistorySeen(
  client: ContractsClient,
  queryClient: QueryClient,
  event: Extract<
    ScheduledTaskEvent,
    { kind: "run_finished" | "occurrence_missed" }
  >,
): Promise<boolean> {
  return serializeDedupTurn(async () => {
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
      // The event remains actionable even when its history refresh is temporarily
      // unavailable, so it surfaces without being claimed here. The later startup
      // pass may repeat it — a registered trade-off: a duplicate beats a silent loss.
      return true;
    }
  });
}

let startupSurfaceInFlight: Promise<void> | null = null;
let startupSurfaceQueuedPromise: Promise<void> | null = null;

/**
 * Surfaces persisted Scheduled Task facts that were recorded while the shell could not
 * receive application events: startup-recovered missed occurrences and interrupted failed
 * runs. Exact per-fact keys prevent duplicate notifications; the first run on an install
 * adopts the existing history as already seen instead of replaying it as a notification
 * storm.
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
  const surface = serializeDedupTurn(() =>
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
  // The very first run on an install adopts the existing history as already seen instead
  // of replaying every historical failure as a notification storm. Subsequent passes
  // only surface facts whose exact key has not been claimed — no timestamp watermark is
  // involved, so facts committed out of clock order across tasks can never be silently
  // absorbed by another task's advancement.
  const migrateOnly = !seenFactsInitialized();

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
      continue;
    }
    for (const entry of entries) {
      const factAt = surfaceableFactAt(entry);
      const key = factKey(task.id, entry);
      if (factAt === null || key === null || hasSeenFact(key)) continue;
      if (migrateOnly) {
        // First run on an install: adopt every existing fact as seen without
        // surfacing, so the user is not notified about failures that predate
        // the feature's activation.
        markFactSeen(key, factAt);
        continue;
      }
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
      markFactSeen(key, factAt);
    }
  }

  markSeenFactsInitialized();
}
