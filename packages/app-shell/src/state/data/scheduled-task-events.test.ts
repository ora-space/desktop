import { beforeEach, describe, expect, it, vi } from "vitest";
import { toast } from "@ora/ui";
import type {
  ScheduledTask,
  ScheduledTaskEvent,
  ScheduledTaskHistoryEntry,
} from "@ora/contracts";
import { createTestClient } from "../../test/contracts-transport";
import { createTestQueryClient } from "../../test/hook-harness";
import type { PlatformAdapter } from "../../platform";
import { scheduledTaskKeys } from "../hooks/use-scheduled-tasks";
import { useUiStore } from "../stores/ui-store";
import { useWorkspaceSelectionStore } from "../stores/workspace-selection-store";
import { surfaceScheduledTaskEvent } from "./scheduled-task-events";
import { markScheduledTaskHistorySeen } from "./scheduled-task-startup-surface";

function failedHistory(id: string): ScheduledTaskHistoryEntry {
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
    updatedAt: 5_000n,
  };
}

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

async function surface(
  platform: PlatformAdapter | null,
  event: ScheduledTaskEvent,
): Promise<void> {
  const queryClient = createTestQueryClient();
  queryClient.setQueryData(scheduledTaskKeys.all, [TASK]);
  const client = createTestClient({
    listScheduledTasks: async () => ({ tasks: [TASK] }),
  });
  await surfaceScheduledTaskEvent(platform, event, queryClient, client);
}

function setVisibility(state: DocumentVisibilityState): void {
  Object.defineProperty(document, "visibilityState", {
    value: state,
    configurable: true,
  });
}

describe("scheduled task event surfacing", () => {
  let successToast: ReturnType<typeof vi.spyOn>;
  let errorToast: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    vi.restoreAllMocks();
    successToast = vi.spyOn(toast, "success").mockImplementation(() => "id");
    errorToast = vi.spyOn(toast, "error").mockImplementation(() => "id");
    setVisibility("visible");
    useUiStore.setState({ scheduledTasksOpen: false });
    useWorkspaceSelectionStore.getState().clearSelection();
  });

  it("announces a started run with a toast that navigates to the new session", async () => {
    const { platform, show } = createPlatform();

    await surface(platform, {
      kind: "run_started",
      taskId: "scheduled-1",
      sessionId: "session-1",
      worktreeTaskId: "worktree-task-1",
      projectId: "project-1",
    });

    expect(successToast).toHaveBeenCalledTimes(1);
    const action = successToast.mock.calls[0]?.[1]?.action;
    expect(action).toBeDefined();
    action?.onClick();
    const selection = useWorkspaceSelectionStore.getState().selection;
    expect(selection).toMatchObject({
      sessionId: "session-1",
      taskId: "worktree-task-1",
      projectId: "project-1",
    });
    expect(useUiStore.getState().scheduledTasksOpen).toBe(false);
    expect(show).not.toHaveBeenCalled();
  });

  it("sends the OS notification for a started run only while the window is hidden", async () => {
    setVisibility("hidden");
    const { platform, show } = createPlatform();

    await surface(platform, {
      kind: "run_started",
      taskId: "scheduled-1",
      sessionId: "session-1",
      worktreeTaskId: "worktree-task-1",
      projectId: "project-1",
    });

    expect(show).toHaveBeenCalledTimes(1);
    expect(show.mock.calls[0]?.[0]?.title).toContain("定时任务已开始");
    // The hidden-window toast must not fire yet, or it would expire unseen.
    expect(successToast).not.toHaveBeenCalled();

    // Regaining focus still yields the navigation entry the OS notification cannot carry.
    setVisibility("visible");
    document.dispatchEvent(new Event("visibilitychange"));
    await Promise.resolve();
    expect(successToast).toHaveBeenCalledTimes(1);
    const action = successToast.mock.calls[0]?.[1]?.action;
    action?.onClick();
    expect(useWorkspaceSelectionStore.getState().selection).toMatchObject({
      sessionId: "session-1",
      taskId: "worktree-task-1",
      projectId: "project-1",
    });
  });

  it("keeps a failed fact's navigation entry until the window is visible again", async () => {
    setVisibility("hidden");
    const { platform, show } = createPlatform();

    await surface(platform, {
      kind: "run_finished",
      taskId: "scheduled-1",
      runId: "run-1",
      status: "failed",
      failureReason: "timeout",
    });

    expect(show).toHaveBeenCalledTimes(1);
    expect(errorToast).not.toHaveBeenCalled();

    setVisibility("visible");
    document.dispatchEvent(new Event("visibilitychange"));
    await Promise.resolve();
    expect(errorToast).toHaveBeenCalledTimes(1);
    expect(errorToast.mock.calls[0]?.[1]?.description).toContain(
      "失败：运行超时",
    );
  });

  it("announces a failed run without notifying successful or cancelled runs", async () => {
    const { platform, show } = createPlatform();

    await surface(platform, {
      kind: "run_finished",
      taskId: "scheduled-1",
      runId: "run-1",
      status: "failed",
      failureReason: "timeout",
    });
    expect(errorToast).toHaveBeenCalledTimes(1);
    expect(errorToast.mock.calls[0]?.[1]?.description).toContain(
      "失败：运行超时",
    );

    await surface(platform, {
      kind: "run_finished",
      taskId: "scheduled-1",
      runId: "run-1",
      status: "succeeded",
      failureReason: null,
    });
    expect(successToast).not.toHaveBeenCalled();
    expect(errorToast).toHaveBeenCalledTimes(1);
    expect(show).not.toHaveBeenCalled();
  });

  it("never toasts the same failed fact twice", async () => {
    const { platform } = createPlatform();
    const queryClient = createTestQueryClient();
    const client = createTestClient({
      listScheduledTaskHistory: async () => ({
        entries: [failedHistory("run-1")],
      }),
    });
    const failedEvent = {
      kind: "run_finished",
      taskId: "scheduled-1",
      runId: "run-1",
      status: "failed",
      failureReason: "timeout",
    } as const;

    // The hook order claims the fact through the history-backed ledger before surfacing;
    // a second delivery of the same event claims nothing and must stay silent.
    for (let delivery = 0; delivery < 2; delivery += 1) {
      const shouldSurface = await markScheduledTaskHistorySeen(
        client,
        queryClient,
        failedEvent,
      );
      if (shouldSurface) {
        await surfaceScheduledTaskEvent(
          platform,
          failedEvent,
          queryClient,
          client,
        );
      }
    }

    expect(errorToast).toHaveBeenCalledTimes(1);
  });

  it("announces a missed occurrence and deep-links to the task history", async () => {
    const { platform, show } = createPlatform();

    await surface(platform, {
      kind: "occurrence_missed",
      taskId: "scheduled-1",
      scheduledAt: 1_000n,
      reason: "app_not_running",
    });

    expect(errorToast).toHaveBeenCalledTimes(1);
    const action = errorToast.mock.calls[0]?.[1]?.action;
    action?.onClick();
    expect(useUiStore.getState().scheduledTasksOpen).toBe(true);
    expect(useUiStore.getState().scheduledTaskHistoryRequest).toEqual({
      taskId: "scheduled-1",
    });
    expect(show).not.toHaveBeenCalled();
  });
});
