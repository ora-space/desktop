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
  worktreeBaseBranch: "main",
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

/** Like missedFact but with a distinct scheduledAt so several facts can share one task. */
function missedAt(scheduledAt: bigint): ScheduledTaskHistoryEntry {
  return {
    kind: "missed",
    scheduledAt,
    reason: "app_not_running",
    observedAt: scheduledAt,
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
    verdict: null,
    verdictDetail: null,
    verdictAt: null,
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

  it("surfaces every unseen fact regardless of the legacy baseline", async () => {
    window.localStorage.setItem("ora.scheduledTaskFactsSeenAt", "3000");
    const { platform } = createPlatform();
    const storage = harness([failedRun(3_000n), failedRun(4_000n, "run-2")]);

    await surfaceStartupScheduledFacts(
      storage.client,
      storage.queryClient,
      platform,
    );

    // Exact-key suppression is the only suppressor: facts the bounded seen map
    // never recorded surface even below the legacy baseline, because a
    // timestamp never proves they were surfaced before. The baseline still
    // advances as bookkeeping for the adopted era.
    expect(errorToast).toHaveBeenCalledTimes(2);
    expect(errorToast.mock.calls[1]?.[1]?.description).toContain(
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

  it("re-surfaces an evicted legacy fact exactly once and re-marks it seen", async () => {
    window.localStorage.setItem("ora.scheduledTaskFactsSeenAt", "5000");
    window.localStorage.setItem("ora.scheduledTaskFactsSeenInitialized", "1");
    const { platform } = createPlatform();
    const storage = harness([failedRun(4_000n, "run-1")]);

    await surfaceStartupScheduledFacts(
      storage.client,
      storage.queryClient,
      platform,
    );

    // The bounded seen map is the only suppressor: a fact it never recorded
    // (evicted or legacy) surfaces once and is re-marked seen — the registered
    // duplicate-over-loss trade-off.
    expect(errorToast).toHaveBeenCalledTimes(1);
    expect(window.localStorage.getItem("ora.scheduledTaskFactsSeen")).toContain(
      "run:scheduled-1:run-1",
    );

    await surfaceStartupScheduledFacts(
      storage.client,
      storage.queryClient,
      platform,
    );
    expect(errorToast).toHaveBeenCalledTimes(1);
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

  describe("dedupe edge cases from circus#333", () => {
    afterEach(() => {
      vi.useRealTimers();
    });

    it("surfaces facts recorded after a failed first migration pass instead of adopting them", async () => {
      vi.useFakeTimers({ toFake: ["Date"] });
      vi.setSystemTime(10_000);
      window.localStorage.clear();
      const oldMissed = missedAt(8_000n);
      const interimMissed = missedAt(12_000n);
      const bMissed = missedAt(9_000n);
      let taskAReads = 0;
      const handlers: TestHandlers = {
        listScheduledTasks: async () => ({ tasks: [TASK, TASK_B] }),
        listScheduledTaskHistory: async ({ taskId }) => {
          if (taskId === TASK.id) {
            taskAReads += 1;
            // The first migration pass cannot read task A, so the initialized marker
            // stays unwritten and a later pass repeats migration.
            if (taskAReads === 1) throw new Error("history unavailable");
            return { entries: [oldMissed, interimMissed] };
          }
          return { entries: [bMissed] };
        },
      };
      const client = createTestClient(handlers);
      const queryClient = createTestQueryClient();
      const { platform } = createPlatform();

      await surfaceStartupScheduledFacts(client, queryClient, platform);
      expect(
        window.localStorage.getItem("ora.scheduledTaskFactsSeenInitialized"),
      ).toBeNull();
      expect(
        window.localStorage.getItem("ora.scheduledTaskFactsFirstPassStartedAt"),
      ).toBe("10000");

      // The repeat migration pass adopts task A's pre-cutoff fact but must surface the
      // fact recorded after the first pass began instead of silently adopting it.
      await surfaceStartupScheduledFacts(client, queryClient, platform);
      expect(
        window.localStorage.getItem("ora.scheduledTaskFactsSeenInitialized"),
      ).toBe("1");
      expect(
        window.localStorage.getItem("ora.scheduledTaskFactsFirstPassStartedAt"),
      ).toBe("10000");
      expect(errorToast).toHaveBeenCalledTimes(1);
      expect(toastCalls(errorToast)[0]?.description).toContain("已错过");
      const seen =
        window.localStorage.getItem("ora.scheduledTaskFactsSeen") ?? "";
      expect(seen).toContain("missed:scheduled-1:8000");
      expect(seen).toContain("missed:scheduled-1:12000");
      expect(seen).toContain("missed:scheduled-2:9000");

      await surfaceStartupScheduledFacts(client, queryClient, platform);
      expect(errorToast).toHaveBeenCalledTimes(1);
    });

    it("still surfaces a fact that lands after the pass read its task when later facts race the baseline", async () => {
      vi.useFakeTimers({ toFake: ["Date"] });
      vi.setSystemTime(10_000);
      const firstMissed = missedAt(5_000n);
      const lateMissed = missedAt(12_000n);
      const racingFailed = failedRun(13_000n);
      let taskAReads = 0;
      const handlers: TestHandlers = {
        listScheduledTasks: async () => ({ tasks: [TASK, TASK_B] }),
        listScheduledTaskHistory: async ({ taskId }) => {
          if (taskId === TASK.id) {
            taskAReads += 1;
            // The late fact is recorded after the first pass has already read task A,
            // so only the second pass can observe it.
            return {
              entries:
                taskAReads === 1 ? [firstMissed] : [firstMissed, lateMissed],
            };
          }
          return { entries: [racingFailed] };
        },
      };
      const client = createTestClient(handlers);
      const queryClient = createTestQueryClient();
      const { platform } = createPlatform();

      await surfaceStartupScheduledFacts(client, queryClient, platform);
      // The baseline must not race past the late fact: it advances only to the pass's
      // start, not to the newest fact the pass happened to observe.
      expect(window.localStorage.getItem("ora.scheduledTaskFactsSeenAt")).toBe(
        "10000",
      );

      await surfaceStartupScheduledFacts(client, queryClient, platform);
      expect(errorToast).toHaveBeenCalledTimes(3);
      const descriptions = toastCalls(errorToast).map(
        (call) => call.description ?? "",
      );
      expect(
        descriptions.filter((description) => description.includes("已错过")),
      ).toHaveLength(2);
      expect(
        window.localStorage.getItem("ora.scheduledTaskFactsSeen") ?? "",
      ).toContain("missed:scheduled-1:12000");

      await surfaceStartupScheduledFacts(client, queryClient, platform);
      expect(errorToast).toHaveBeenCalledTimes(3);
    });

    it("surfaces a live fact when its history refresh fails and leaves it for the backfill pass", async () => {
      const failing = createTestClient({
        listScheduledTasks: async () => ({ tasks: [TASK] }),
        listScheduledTaskHistory: async () => {
          throw new Error("history unavailable");
        },
      });
      const queryClient = createTestQueryClient();

      const shouldSurface = await markScheduledTaskHistorySeen(
        failing,
        queryClient,
        {
          kind: "run_finished",
          taskId: TASK.id,
          runId: "run-1",
          status: "failed",
          failureReason: "timeout",
        },
      );
      expect(shouldSurface).toBe(true);
      expect(
        window.localStorage.getItem("ora.scheduledTaskFactsSeen") ?? "",
      ).not.toContain("run:scheduled-1:run-1");

      // The registered trade-off: the unclaimed fact repeats on the next successful
      // startup pass, because a duplicate beats a silent loss.
      const { platform } = createPlatform();
      const storage = harness([failedRun(5_000n)]);
      await surfaceStartupScheduledFacts(
        storage.client,
        storage.queryClient,
        platform,
      );
      expect(errorToast).toHaveBeenCalledTimes(1);
      expect(toastCalls(errorToast)[0]?.description).toContain(
        "失败：运行超时",
      );
    });

    it("suppresses a live fact whose history refresh fails after it was already claimed", async () => {
      let historyFails = false;
      const handlers: TestHandlers = {
        listScheduledTasks: async () => ({ tasks: [TASK] }),
        listScheduledTaskHistory: async () => {
          if (historyFails) throw new Error("history unavailable");
          return { entries: [failedRun(5_000n)] };
        },
      };
      const client = createTestClient(handlers);
      const queryClient = createTestQueryClient();
      const { platform } = createPlatform();

      // The startup pass surfaces and claims the fact first.
      await surfaceStartupScheduledFacts(client, queryClient, platform);
      expect(errorToast).toHaveBeenCalledTimes(1);

      historyFails = true;
      const shouldSurface = await markScheduledTaskHistorySeen(
        client,
        queryClient,
        {
          kind: "run_finished",
          taskId: TASK.id,
          runId: "run-1",
          status: "failed",
          failureReason: "timeout",
        },
      );
      expect(shouldSurface).toBe(false);
      expect(errorToast).toHaveBeenCalledTimes(1);
    });

    it("does not lose a post-cutoff fact across two failed migration reads", async () => {
      vi.useFakeTimers({ toFake: ["Date"] });
      vi.setSystemTime(10_000);
      window.localStorage.clear();
      let pass = 0;
      const client = createTestClient({
        listScheduledTasks: async () => {
          pass++;
          return { tasks: [TASK, TASK_B] };
        },
        listScheduledTaskHistory: async ({ taskId }) => {
          if (taskId === TASK.id) {
            if (pass < 3) throw new Error("A unavailable");
            return { entries: [missedAt(12_000n)] };
          }
          return { entries: [missedAt(pass === 1 ? 8_000n : 15_000n)] };
        },
      });
      const queryClient = createTestQueryClient();
      const { platform } = createPlatform();
      await surfaceStartupScheduledFacts(client, queryClient, platform);
      vi.setSystemTime(20_000);
      await surfaceStartupScheduledFacts(client, queryClient, platform);
      expect(errorToast).toHaveBeenCalledTimes(1);
      expect(
        window.localStorage.getItem("ora.scheduledTaskFactsFirstPassStartedAt"),
      ).toBe("10000");
      expect(window.localStorage.getItem("ora.scheduledTaskFactsSeenAt")).toBe(
        "15000",
      );
      vi.setSystemTime(30_000);
      await surfaceStartupScheduledFacts(client, queryClient, platform);
      expect(errorToast).toHaveBeenCalledTimes(2);
    });

    it("surfaces a late commit with a timestamp sampled before the pass began", async () => {
      vi.useFakeTimers({ toFake: ["Date"] });
      vi.setSystemTime(10_000);
      let reads = 0;
      const client = createTestClient({
        listScheduledTasks: async () => ({ tasks: [TASK, TASK_B] }),
        listScheduledTaskHistory: async ({ taskId }) => {
          if (taskId === TASK.id) {
            reads++;
            return {
              entries:
                reads === 1
                  ? [missedAt(5_000n)]
                  : [missedAt(5_000n), missedAt(9_999n)],
            };
          }
          return { entries: [failedRun(13_000n)] };
        },
      });
      const queryClient = createTestQueryClient();
      const { platform } = createPlatform();
      await surfaceStartupScheduledFacts(client, queryClient, platform);
      expect(errorToast).toHaveBeenCalledTimes(2);
      expect(window.localStorage.getItem("ora.scheduledTaskFactsSeenAt")).toBe(
        "10000",
      );
      vi.setSystemTime(20_000);
      await surfaceStartupScheduledFacts(client, queryClient, platform);
      expect(errorToast).toHaveBeenCalledTimes(3);
    });

    it("surfaces every unseen fact in a descending history batch", async () => {
      const storage = harness([missedAt(6_000n), missedAt(5_000n)]);
      const { platform } = createPlatform();
      await surfaceStartupScheduledFacts(
        storage.client,
        storage.queryClient,
        platform,
      );
      expect(errorToast).toHaveBeenCalledTimes(2);
    });

    it("surfaces a same-task late commit older than its observed maximum", async () => {
      let reads = 0;
      const client = createTestClient({
        listScheduledTasks: async () => ({ tasks: [TASK] }),
        listScheduledTaskHistory: async () => ({
          entries:
            ++reads === 1
              ? [missedAt(6_000n)]
              : [missedAt(6_000n), missedAt(5_000n)],
        }),
      });
      const queryClient = createTestQueryClient();
      const { platform } = createPlatform();
      await surfaceStartupScheduledFacts(client, queryClient, platform);
      expect(errorToast).toHaveBeenCalledTimes(1);
      await surfaceStartupScheduledFacts(client, queryClient, platform);
      expect(errorToast).toHaveBeenCalledTimes(2);
    });

    it("does not adopt a task's first successful read under another task's baseline", async () => {
      let pass = 0;
      const client = createTestClient({
        listScheduledTasks: async () => {
          pass++;
          return { tasks: [TASK, TASK_B] };
        },
        listScheduledTaskHistory: async ({ taskId }) => {
          if (taskId === TASK.id) {
            if (pass === 1) throw new Error("A unavailable");
            return { entries: [missedAt(5_000n)] };
          }
          return { entries: [missedAt(6_000n)] };
        },
      });
      const queryClient = createTestQueryClient();
      const { platform } = createPlatform();
      await surfaceStartupScheduledFacts(client, queryClient, platform);
      expect(errorToast).toHaveBeenCalledTimes(1);
      await surfaceStartupScheduledFacts(client, queryClient, platform);
      expect(errorToast).toHaveBeenCalledTimes(2);
    });
  });
});
