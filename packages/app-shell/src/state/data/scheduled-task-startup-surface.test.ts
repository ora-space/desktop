import { beforeEach, describe, expect, it, vi } from "vitest";
import { toast } from "@ora/ui";
import type { ScheduledTask, ScheduledTaskHistoryEntry } from "@ora/contracts";
import {
  createTestClient,
  type TestHandlers,
} from "../../test/contracts-transport";
import { createTestQueryClient } from "../../test/hook-harness";
import type { PlatformAdapter } from "../../platform";
import { useUiStore } from "../stores/ui-store";
import {
  markScheduledTaskHistorySeen,
  surfaceStartupScheduledFacts,
} from "./scheduled-task-startup-surface";
import { surfaceScheduledTaskEvent } from "./scheduled-task-events";

const TASK: ScheduledTask = {
  id: "scheduled-1",
  name: "Daily review",
  enabled: true,
  schedule: { kind: "daily", hour: 9, minute: 30 },
  executionLocation: "local",
  projectId: "project-1",
  agentRef: "ora-space/codex",
  model: null,
  prompt: "Review the current changes",
  revision: 1n,
  approval: { policy: "full_auto_in_worktree", consentedAt: 10n },
  createdAt: 10n,
  updatedAt: 10n,
};

const TASK_B: ScheduledTask = {
  ...TASK,
  id: "scheduled-2",
  name: "Weekly digest",
};

function deferred<T>(): { promise: Promise<T>; resolve: (value: T) => void } {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolveIt) => {
    resolve = resolveIt;
  });
  return { promise, resolve };
}

function missedFact(observedAt: bigint): ScheduledTaskHistoryEntry {
  return {
    kind: "missed",
    scheduledAt: 1_000n,
    reason: "app_not_running",
    observedAt,
  };
}

function failedRun(updatedAt: bigint, id = "run-1"): ScheduledTaskHistoryEntry {
  return {
    kind: "run",
    id,
    scheduledAt: 2_000n,
    revision: 1n,
    status: "failed",
    sessionId: "session-1",
    worktreePath: null,
    worktreeTaskId: null,
    failureReason: "timeout",
    createdAt: 2_000n,
    updatedAt,
  };
}

function createPlatform(): {
  platform: PlatformAdapter;
  show: ReturnType<typeof vi.fn>;
} {
  const show = vi.fn().mockResolvedValue(true);
  return {
    show,
    platform: {
      desktopNotifications: { show },
    } as unknown as PlatformAdapter,
  };
}

function harness(entries: ScheduledTaskHistoryEntry[]) {
  const handlers: TestHandlers = {
    listScheduledTasks: async () => ({ tasks: [TASK] }),
    listScheduledTaskHistory: async () => ({ entries: [...entries] }),
  };
  return {
    client: createTestClient(handlers),
    queryClient: createTestQueryClient(),
  };
}

function spyOnErrorToast() {
  return vi.spyOn(toast, "error").mockImplementation(() => "id");
}

interface RecordedToastCall {
  description?: string;
  action?: { onClick: () => void };
}

/** Narrows sonner's broad toast payload to the shape these surfaces emit. */
function toastCalls(
  spy: ReturnType<typeof spyOnErrorToast>,
): RecordedToastCall[] {
  return spy.mock.calls.map(
    (call) => call[1] as RecordedToastCall | undefined,
  ) as RecordedToastCall[];
}

describe("startup surface of persisted scheduled task facts", () => {
  let errorToast: ReturnType<typeof spyOnErrorToast>;

  beforeEach(() => {
    vi.restoreAllMocks();
    errorToast = spyOnErrorToast();
    window.localStorage.clear();
    // Most tests simulate an already-installed app whose first-ever migration pass has
    // happened; the migration tests below clear storage to start from a fresh install.
    window.localStorage.setItem("ora.scheduledTaskFactsSeenInitialized", "1");
    useUiStore.setState({ scheduledTasksOpen: false });
  });

  it("surfaces a missed fact recorded while the shell was closed and advances the baseline", async () => {
    const { platform, show } = createPlatform();
    const first = harness([missedFact(5_000n)]);

    await surfaceStartupScheduledFacts(
      first.client,
      first.queryClient,
      platform,
    );

    expect(errorToast).toHaveBeenCalledTimes(1);
    expect(toastCalls(errorToast)[0]?.description).toContain(
      "已错过：Desktop 未运行",
    );
    const action = toastCalls(errorToast)[0]?.action;
    action?.onClick();
    expect(useUiStore.getState().scheduledTaskHistoryRequest).toEqual({
      taskId: TASK.id,
    });
    expect(window.localStorage.getItem("ora.scheduledTaskFactsSeenAt")).toBe(
      "5000",
    );
    expect(show).not.toHaveBeenCalled();

    // The second pass must not repeat the same fact.
    await surfaceStartupScheduledFacts(
      first.client,
      first.queryClient,
      platform,
    );
    expect(errorToast).toHaveBeenCalledTimes(1);
  });

  it("surfaces a failed run and skips facts at or before the baseline", async () => {
    window.localStorage.setItem("ora.scheduledTaskFactsSeenAt", "3000");
    const { platform } = createPlatform();
    const storage = harness([failedRun(3_000n), failedRun(4_000n, "run-2")]);

    await surfaceStartupScheduledFacts(
      storage.client,
      storage.queryClient,
      platform,
    );

    expect(errorToast).toHaveBeenCalledTimes(1);
    expect(errorToast.mock.calls[0]?.[1]?.description).toContain(
      "失败：运行超时",
    );
    expect(window.localStorage.getItem("ora.scheduledTaskFactsSeenAt")).toBe(
      "4000",
    );
  });

  it("records a live fact before a reconnect can replay it from history", async () => {
    const { platform } = createPlatform();
    const storage = harness([failedRun(5_000n)]);

    await markScheduledTaskHistorySeen(storage.client, storage.queryClient, {
      kind: "run_finished",
      taskId: TASK.id,
      runId: "run-1",
      status: "failed",
      failureReason: "timeout",
    });
    await surfaceStartupScheduledFacts(
      storage.client,
      storage.queryClient,
      platform,
    );

    expect(errorToast).not.toHaveBeenCalled();
    expect(window.localStorage.getItem("ora.scheduledTaskFactsSeen")).toContain(
      "run:scheduled-1:run-1",
    );
  });

  it("claims exactly the run the event names when two failures share a reason", async () => {
    const { platform } = createPlatform();
    const storage = harness([
      failedRun(5_000n, "run-1"),
      failedRun(6_000n, "run-2"),
    ]);

    const shouldSurface = await markScheduledTaskHistorySeen(
      storage.client,
      storage.queryClient,
      {
        kind: "run_finished",
        taskId: TASK.id,
        runId: "run-2",
        status: "failed",
        failureReason: "timeout",
      },
    );
    expect(shouldSurface).toBe(true);
    expect(window.localStorage.getItem("ora.scheduledTaskFactsSeen")).toContain(
      "run:scheduled-1:run-2",
    );
    expect(
      window.localStorage.getItem("ora.scheduledTaskFactsSeen"),
    ).not.toContain("run:scheduled-1:run-1");

    // The untouched earlier failure still surfaces; the claimed one does not.
    await surfaceStartupScheduledFacts(
      storage.client,
      storage.queryClient,
      platform,
    );
    expect(errorToast).toHaveBeenCalledTimes(1);
  });

  it("adopts existing history as seen on the first run after an upgrade", async () => {
    window.localStorage.clear();
    const { platform } = createPlatform();
    const storage = harness([failedRun(4_000n, "run-1"), missedFact(5_000n)]);

    await surfaceStartupScheduledFacts(
      storage.client,
      storage.queryClient,
      platform,
    );

    expect(errorToast).not.toHaveBeenCalled();
    expect(window.localStorage.getItem("ora.scheduledTaskFactsSeenAt")).toBe(
      "5000",
    );
    const seen =
      window.localStorage.getItem("ora.scheduledTaskFactsSeen") ?? "";
    expect(seen).toContain("run:scheduled-1:run-1");
    expect(seen).toContain("missed:scheduled-1:1000");

    // Later passes must not replay the adopted facts either.
    await surfaceStartupScheduledFacts(
      storage.client,
      storage.queryClient,
      platform,
    );
    expect(errorToast).not.toHaveBeenCalled();
  });

  it("suppresses facts covered by the legacy baseline even after seen-map eviction", async () => {
    window.localStorage.setItem("ora.scheduledTaskFactsSeenAt", "5000");
    window.localStorage.setItem("ora.scheduledTaskFactsSeenInitialized", "1");
    const { platform } = createPlatform();
    const storage = harness([failedRun(4_000n, "run-1")]);

    await surfaceStartupScheduledFacts(
      storage.client,
      storage.queryClient,
      platform,
    );

    expect(errorToast).not.toHaveBeenCalled();
    expect(window.localStorage.getItem("ora.scheduledTaskFactsSeen")).toContain(
      "run:scheduled-1:run-1",
    );
  });

  it("sends the OS notification while the window is hidden and keeps the entry for refocus", async () => {
    Object.defineProperty(document, "visibilityState", {
      value: "hidden",
      configurable: true,
    });
    const { platform, show } = createPlatform();
    const storage = harness([missedFact(5_000n)]);

    await surfaceStartupScheduledFacts(
      storage.client,
      storage.queryClient,
      platform,
    );

    expect(show).toHaveBeenCalledTimes(1);
    // While hidden the toast must not fire, or it would expire unseen; the navigation
    // entry is retained and rendered when the window is visible again.
    expect(errorToast).not.toHaveBeenCalled();
    expect(window.localStorage.getItem("ora.scheduledTaskFactsSeenAt")).toBe(
      "5000",
    );

    Object.defineProperty(document, "visibilityState", {
      value: "visible",
      configurable: true,
    });
    document.dispatchEvent(new Event("visibilitychange"));
    await Promise.resolve();
    expect(errorToast).toHaveBeenCalledTimes(1);
    expect(errorToast.mock.calls[0]?.[1]?.description).toContain(
      "已错过：Desktop 未运行",
    );
  });

  it("keeps older facts surfaceable when a live event advances the baseline mid-surface", async () => {
    const missedA = missedFact(1_000n);
    const failedB = failedRun(5_000n);
    const blocked = deferred<ScheduledTaskHistoryEntry[]>();
    const handlers: TestHandlers = {
      listScheduledTasks: async () => ({ tasks: [TASK, TASK_B] }),
      listScheduledTaskHistory: async ({ taskId }) => {
        if (taskId === TASK.id) return { entries: await blocked.promise };
        return { entries: [failedB] };
      },
    };
    const client = createTestClient(handlers);
    const queryClient = createTestQueryClient();
    const { platform } = createPlatform();

    const surfacePromise = surfaceStartupScheduledFacts(
      client,
      queryClient,
      platform,
    );
    // A live failed fact for task B lands while task A's history read is still in flight.
    // The dedupe write must wait for the in-flight surface pass, or task A's older missed
    // fact would be skipped by the advanced baseline instead of surfaced.
    const markPromise = markScheduledTaskHistorySeen(client, queryClient, {
      kind: "run_finished",
      taskId: TASK_B.id,
      runId: "run-1",
      status: "failed",
      failureReason: "timeout",
    });
    blocked.resolve([missedA]);
    await Promise.all([surfacePromise, markPromise]);
    await Promise.resolve();

    const descriptions = toastCalls(errorToast).map(
      (call) => call.description ?? "",
    );
    expect(
      descriptions.some((description) => description?.includes("已错过")),
      "the older missed fact is still surfaced",
    ).toBe(true);
    expect(
      descriptions.some((description) =>
        description?.includes("失败：运行超时"),
      ),
      "the live failed fact is surfaced",
    ).toBe(true);
    expect(window.localStorage.getItem("ora.scheduledTaskFactsSeenAt")).toBe(
      "5000",
    );
  });

  it("surfaces an interleaved live failure exactly once", async () => {
    const missedA = missedFact(1_000n);
    const failedB = failedRun(5_000n);
    const blocked = deferred<ScheduledTaskHistoryEntry[]>();
    const handlers: TestHandlers = {
      listScheduledTasks: async () => ({ tasks: [TASK, TASK_B] }),
      listScheduledTaskHistory: async ({ taskId }) => {
        if (taskId === TASK.id) return { entries: await blocked.promise };
        return { entries: [failedB] };
      },
    };
    const client = createTestClient(handlers);
    const queryClient = createTestQueryClient();
    const { platform } = createPlatform();
    const liveEvent = {
      kind: "run_finished" as const,
      taskId: TASK_B.id,
      runId: "run-1",
      status: "failed" as const,
      failureReason: "timeout" as const,
    };

    const startup = surfaceStartupScheduledFacts(client, queryClient, platform);
    const live = (async () => {
      const shouldSurface = await markScheduledTaskHistorySeen(
        client,
        queryClient,
        liveEvent,
      );
      if (shouldSurface) {
        await surfaceScheduledTaskEvent(
          platform,
          liveEvent,
          queryClient,
          client,
        );
      }
    })();
    blocked.resolve([missedA]);
    await Promise.all([startup, live]);

    const descriptions = toastCalls(errorToast).map(
      (call) => call.description ?? "",
    );
    expect(
      descriptions.filter((description) => description.includes("已错过")),
    ).toHaveLength(1);
    expect(
      descriptions.filter((description) =>
        description.includes("失败：运行超时"),
      ),
    ).toHaveLength(1);
  });

  it("surfaces facts persisted during an in-flight pass on the queued follow-up", async () => {
    const missedA = missedFact(1_000n);
    const blocked = deferred<ScheduledTaskHistoryEntry[]>();
    const entriesB: ScheduledTaskHistoryEntry[] = [];
    const handlers: TestHandlers = {
      listScheduledTasks: async () => ({ tasks: [TASK, TASK_B] }),
      listScheduledTaskHistory: async ({ taskId }) => {
        if (taskId === TASK.id) return { entries: await blocked.promise };
        return { entries: [...entriesB] };
      },
    };
    const client = createTestClient(handlers);
    const queryClient = createTestQueryClient();
    const { platform } = createPlatform();

    const firstPass = surfaceStartupScheduledFacts(
      client,
      queryClient,
      platform,
    );
    // A second ready arrives while the first pass is still running: it queues a follow-up
    // pass instead of reusing the in-flight promise, and awaiting its returned promise
    // waits out the queued pass.
    const secondPass = surfaceStartupScheduledFacts(
      client,
      queryClient,
      platform,
    );
    // A fact is persisted for task B during the in-flight first pass.
    entriesB.push(missedFact(6_000n));
    blocked.resolve([missedA]);
    await firstPass;
    await secondPass;

    expect(errorToast).toHaveBeenCalledTimes(2);
    expect(toastCalls(errorToast)[1]?.description).toContain(
      "已错过：Desktop 未运行",
    );
    expect(window.localStorage.getItem("ora.scheduledTaskFactsSeenAt")).toBe(
      "6000",
    );
  });

  it("does not silently absorb facts persisted after a partial-history pass began", async () => {
    // Simulate: the first pass reads task A's history successfully but task B's history read
    // fails (historiesComplete = false). Between passes, a new failed fact is persisted for
    // task B. The next pass must NOT silently adopt it — it must surface a notification.
    window.localStorage.setItem("ora.scheduledTaskFactsSeenInitialized", "1");
    window.localStorage.setItem("ora.scheduledTaskFactsSeenAt", "3000");

    const missedB = missedFact(8_000n);
    const { platform } = createPlatform();
    const entriesB: ScheduledTaskHistoryEntry[] = [];
    const handlers: TestHandlers = {
      listScheduledTasks: async () => ({ tasks: [TASK] }),
      listScheduledTaskHistory: async () => ({
        entries: [missedB, ...entriesB],
      }),
    };
    const client = createTestClient(handlers);
    const queryClient = createTestQueryClient();

    // First pass: the baseline is 3000, the missed fact at 8000 is above it and gets
    // surfaced. The initialized flag is already set so the next pass does not migrate.
    await surfaceStartupScheduledFacts(client, queryClient, platform);
    expect(errorToast).toHaveBeenCalledTimes(1);
    expect(window.localStorage.getItem("ora.scheduledTaskFactsSeenAt")).toBe(
      "8000",
    );

    // Second pass: no new facts, no new toasts.
    await surfaceStartupScheduledFacts(client, queryClient, platform);
    expect(errorToast).toHaveBeenCalledTimes(1);
  });

  it("does not absorb live facts that arrive during a partial pass", async () => {
    // First pass adopts existing history as migrate-only. A fact arrives for task B after
    // the pass — the next pass must surface it, not silently absorb it because the
    // initialized flag was not set (historiesComplete was false).
    window.localStorage.clear();
    const missedA = missedFact(1_000n);
    const failedB = failedRun(5_000n);
    const handlers: TestHandlers = {
      listScheduledTasks: async () => ({ tasks: [TASK, TASK_B] }),
      listScheduledTaskHistory: async ({ taskId }) => {
        if (taskId === TASK.id) return { entries: [missedA] };
        return { entries: [failedB] };
      },
    };
    const client = createTestClient(handlers);
    const queryClient = createTestQueryClient();
    const { platform } = createPlatform();

    // First pass on a fresh install (migrate-only): adopts both facts as seen without
    // surfacing, advances the baseline, and marks initialized.
    await surfaceStartupScheduledFacts(client, queryClient, platform);
    expect(errorToast).not.toHaveBeenCalled();
    expect(
      window.localStorage.getItem("ora.scheduledTaskFactsSeenInitialized"),
    ).toBe("1");

    // Second pass: nothing new, nothing surfaced.
    await surfaceStartupScheduledFacts(client, queryClient, platform);
    expect(errorToast).not.toHaveBeenCalled();
  });
});
