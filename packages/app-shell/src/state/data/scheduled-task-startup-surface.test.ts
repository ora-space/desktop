import { beforeEach, describe, expect, it, vi } from "vitest";
import { toast } from "@ora/ui";
import type { ScheduledTask } from "@ora/contracts";
import { createTestClient } from "../../test/contracts-transport";
import { createTestQueryClient } from "../../test/hook-harness";
import type { PlatformAdapter } from "../../platform";
import { useUiStore } from "../stores/ui-store";
import {
  resetScheduledFactSeen,
  surfaceStartupScheduledFacts,
} from "./scheduled-task-startup-surface";

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

function createPlatform(): PlatformAdapter {
  return {
    desktopNotifications: { show: vi.fn().mockResolvedValue(true) },
  } as unknown as PlatformAdapter;
}

function spyOnErrorToast() {
  return vi.spyOn(toast, "error").mockImplementation(() => "id");
}

describe("startup surface of persisted scheduled task facts", () => {
  let errorToast: ReturnType<typeof spyOnErrorToast>;

  beforeEach(() => {
    vi.restoreAllMocks();
    resetScheduledFactSeen();
    errorToast = spyOnErrorToast();
    window.localStorage.clear();
    useUiStore.setState({ scheduledTasksOpen: false });
  });

  it("surfaces a missed fact and does not repeat it on a second pass", async () => {
    const platform = createPlatform();
    const queryClient = createTestQueryClient();
    const client = createTestClient({
      listScheduledTasks: async () => ({ tasks: [TASK] }),
      listScheduledTaskHistory: async () => ({
        entries: [{
          kind: "missed",
          scheduledAt: 1_000n,
          reason: "app_not_running",
          observedAt: 5_000n,
        }],
      }),
    });

    await surfaceStartupScheduledFacts(client, queryClient, platform);
    expect(errorToast).toHaveBeenCalledTimes(1);

    await surfaceStartupScheduledFacts(client, queryClient, platform);
    expect(errorToast).toHaveBeenCalledTimes(1);
  });

  it("surfaces a failed run and claims its key so a second pass does not repeat it", async () => {
    const platform = createPlatform();
    const queryClient = createTestQueryClient();
    const client = createTestClient({
      listScheduledTasks: async () => ({ tasks: [TASK] }),
      listScheduledTaskHistory: async () => ({
        entries: [
          { kind: "run", id: "run-1", scheduledAt: 2_000n, revision: 1n, status: "failed", sessionId: "session-1", worktreePath: null, worktreeTaskId: null, failureReason: "timeout", createdAt: 2_000n, updatedAt: 5_000n },
        ],
      }),
    });

    await surfaceStartupScheduledFacts(client, queryClient, platform);
    expect(errorToast).toHaveBeenCalledTimes(1);
    expect(toastCalls(errorToast)[0]?.description).toContain("失败：运行超时");

    await surfaceStartupScheduledFacts(client, queryClient, platform);
    expect(errorToast).toHaveBeenCalledTimes(1);
  });

  it("does not silently absorb facts for tasks whose history read failed in an earlier pass", async () => {
    const platform = createPlatform();
    let bFail = true;
    const queryClient = createTestQueryClient();
    const client = createTestClient({
      listScheduledTasks: async () => ({
        tasks: [TASK, { ...TASK, id: "scheduled-2", name: "Weekly digest" }],
      }),
      listScheduledTaskHistory: async ({ taskId }: { taskId: string }) => {
        if (taskId === TASK.id)
          return { entries: [{ kind: "missed", scheduledAt: 1_000n, reason: "app_not_running", observedAt: 1_000n }] };
        if (bFail) throw new Error("transient read failure");
        return { entries: [{ kind: "run", id: "run-B", scheduledAt: 2_000n, revision: 1n, status: "failed", sessionId: "session-B", worktreePath: null, worktreeTaskId: null, failureReason: "timeout", createdAt: 2_000n, updatedAt: 5_000n }] };
      },
    });

    await surfaceStartupScheduledFacts(client, queryClient, platform);
    const aMissCount = toastCalls(errorToast).filter((call) =>
      call.description?.includes("已错过"),
    ).length;
    expect(aMissCount).toBe(1);

    bFail = false;
    await surfaceStartupScheduledFacts(client, queryClient, platform);
    const bFailCount = toastCalls(errorToast).filter((call) =>
      call.description?.includes("失败：运行超时"),
    ).length;
    expect(bFailCount).toBe(1);
  });

  it("adopts existing history as seen on the first run without notification storm", async () => {
    const platform = createPlatform();
    const queryClient = createTestQueryClient();
    const client = createTestClient({
      listScheduledTasks: async () => ({ tasks: [TASK] }),
      listScheduledTaskHistory: async () => ({
        entries: [
          { kind: "missed", scheduledAt: 1_000n, reason: "app_not_running", observedAt: 1_000n },
          { kind: "run", id: "run-1", scheduledAt: 2_000n, revision: 1n, status: "failed", sessionId: "session-1", worktreePath: null, worktreeTaskId: null, failureReason: "timeout", createdAt: 2_000n, updatedAt: 3_000n },
        ],
      }),
    });

    await surfaceStartupScheduledFacts(client, queryClient, platform);
    expect(errorToast).not.toHaveBeenCalled();

    const seen = window.localStorage.getItem("ora.scheduledTaskFactsSeen") ?? "";
    expect(seen).toContain("missed:scheduled-1:1000");
    expect(seen).toContain("run:run-1");
  });
});

function toastCalls(spy: ReturnType<typeof spyOnErrorToast>): RecordedToastCall[] {
  return spy.mock.calls.map(
    (call) => call[1] as RecordedToastCall | undefined,
  ) as RecordedToastCall[];
}

interface RecordedToastCall {
  description?: string;
  action?: { onClick: () => void };
}
