import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
  ScheduledRunVerdict,
  ScheduledTask,
  ScheduledTaskHistoryEntry,
} from "@ora/contracts";
import { createChatStore } from "@ora/chat";
import { TooltipProvider, toast } from "@ora/ui";
import { AppI18nProvider } from "../../i18n/i18n";
import { PlatformProvider } from "../../platform";
import {
  createTestClient,
  type TestHandlers,
} from "../../test/contracts-transport";
import {
  createHookWrapper,
  createTestQueryClient,
} from "../../test/hook-harness";
import { createStubPlatform } from "../../test/stub-platform";
import { useUiStore } from "../../state/stores/ui-store";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import { ScheduledTaskHistoryDialog } from "./scheduled-task-history-dialog";

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

function historyEntry(overrides: {
  id: string;
  status: "succeeded" | "failed" | "running";
  worktreeTaskId: string | null;
  sessionId?: string | null;
  verdict?: ScheduledRunVerdict | null;
}): ScheduledTaskHistoryEntry {
  return {
    kind: "run",
    id: overrides.id,
    scheduledAt: 10n,
    revision: 1n,
    status: overrides.status,
    sessionId: overrides.sessionId ?? null,
    worktreePath: "worktrees/kept",
    worktreeTaskId: overrides.worktreeTaskId,
    failureReason: null,
    verdict: overrides.verdict ?? null,
    verdictDetail: null,
    verdictAt: null,
    createdAt: 10n,
    updatedAt: 15n,
  };
}

interface Harness {
  deletedTaskIds: string[];
  failDiscards?: boolean;
  artifactHandlers?: TestHandlers;
}

function renderDialog(
  entries: ScheduledTaskHistoryEntry[],
  harness: Harness = { deletedTaskIds: [] },
) {
  const worktreeTasks = [
    {
      id: "worktree-task-1",
      // Deliberately different from the definition's project: an old run keeps the
      // project it executed in even after the definition was re-targeted.
      projectId: "project-9",
      workspaceId: "workspace-1",
      title: "Scheduled run run-1",
    },
  ];
  const handlers: TestHandlers = {
    ...harness.artifactHandlers,
    listScheduledTaskHistory: async () => ({ entries: [...entries] }),
    listTasks: async () => ({ tasks: [...worktreeTasks] }),
    deleteTask: async ({ taskId }) => {
      if (harness.failDiscards) {
        throw new Error("scheduled_task_not_found");
      }
      harness.deletedTaskIds.push(taskId);
      const index = worktreeTasks.findIndex((task) => task.id === taskId);
      if (index >= 0) worktreeTasks.splice(index, 1);
      return { taskId, workspaceId: "workspace-1" };
    },
  };
  const client = createTestClient(handlers);
  const Wrapper = createHookWrapper(
    client,
    createTestQueryClient(),
    createChatStore(client.session),
  );
  return render(
    <Wrapper>
      <AppI18nProvider>
        <PlatformProvider adapter={createStubPlatform()}>
          <TooltipProvider>
            <ScheduledTaskHistoryDialog
              task={TASK}
              onOpenChange={() => undefined}
            />
          </TooltipProvider>
        </PlatformProvider>
      </AppI18nProvider>
    </Wrapper>,
  );
}

describe("ScheduledTaskHistoryDialog retained worktrees", () => {
  beforeEach(() => {
    useUiStore.setState({ scheduledTasksOpen: true });
    useWorkspaceSelectionStore.getState().clearSelection();
  });

  it("offers open and discard for a finished run whose worktree task still exists", async () => {
    renderDialog([
      historyEntry({
        id: "run-1",
        status: "succeeded",
        worktreeTaskId: "worktree-task-1",
        sessionId: "session-1",
      }),
    ]);

    expect(await screen.findByRole("button", { name: "打开" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "丢弃" })).toBeTruthy();
  });

  it("shows a business fail verdict without turning the run into a failure", async () => {
    renderDialog([
      historyEntry({
        id: "run-1",
        status: "succeeded",
        worktreeTaskId: null,
        verdict: "fail",
      }),
    ]);

    expect(await screen.findByText("已成功")).toBeTruthy();
    expect(screen.getByText("业务结论: 失败")).toBeTruthy();
    expect(screen.queryByText("失败：执行错误")).toBeNull();
  });

  it("shows no verdict when the run never reported one", async () => {
    renderDialog([
      historyEntry({
        id: "run-1",
        status: "succeeded",
        worktreeTaskId: null,
      }),
    ]);

    await screen.findByText("已成功");
    expect(screen.queryByText(/业务结论/)).toBeNull();
  });

  it("opens the artifact manifest for a succeeded run", async () => {
    const user = userEvent.setup();
    renderDialog(
      [
        historyEntry({
          id: "run-1",
          status: "succeeded",
          worktreeTaskId: null,
        }),
      ],
      {
        deletedTaskIds: [],
        artifactHandlers: {
          listScheduledRunArtifacts: async () => ({
            artifacts: [
              {
                relativePath: "report.md",
                kind: "markdown",
                sizeBytes: 6n,
                registeredAt: 10n,
              },
            ],
            rejections: [],
          }),
        },
      },
    );

    await user.click(await screen.findByRole("button", { name: "运行产物" }));

    expect(await screen.findByText("report.md")).toBeTruthy();
  });

  it("hides the worktree actions while the run is not finished", async () => {
    renderDialog([
      historyEntry({
        id: "run-1",
        status: "running",
        worktreeTaskId: "worktree-task-1",
      }),
    ]);

    await screen.findByText("运行中");
    expect(screen.queryByRole("button", { name: "打开" })).toBeNull();
    expect(screen.queryByRole("button", { name: "丢弃" })).toBeNull();
  });

  it("hides the worktree actions once the task no longer exists", async () => {
    renderDialog([
      historyEntry({
        id: "run-1",
        status: "failed",
        worktreeTaskId: "discarded-task",
      }),
    ]);

    await screen.findByText("失败");
    expect(screen.queryByRole("button", { name: "打开" })).toBeNull();
    expect(screen.queryByRole("button", { name: "丢弃" })).toBeNull();
  });

  it("opening a run leaves the pane and selects its session in the worktree", async () => {
    const user = userEvent.setup();
    renderDialog([
      historyEntry({
        id: "run-1",
        status: "succeeded",
        worktreeTaskId: "worktree-task-1",
        sessionId: "session-1",
      }),
    ]);

    await user.click(await screen.findByRole("button", { name: "打开" }));

    const selection = useWorkspaceSelectionStore.getState().selection;
    expect(selection).toMatchObject({
      sessionId: "session-1",
      taskId: "worktree-task-1",
      // The run's own project, not the definition's current project.
      projectId: "project-9",
    });
    expect(useUiStore.getState().scheduledTasksOpen).toBe(false);
  });

  it("discarding a retained worktree deletes its task after confirmation", async () => {
    const user = userEvent.setup();
    const harness: Harness = { deletedTaskIds: [] };
    renderDialog(
      [
        historyEntry({
          id: "run-1",
          status: "succeeded",
          worktreeTaskId: "worktree-task-1",
        }),
      ],
      harness,
    );

    await user.click(await screen.findByRole("button", { name: "丢弃" }));
    await user.click(await screen.findByRole("button", { name: "丢弃" }));

    await waitFor(() =>
      expect(harness.deletedTaskIds).toEqual(["worktree-task-1"]),
    );
    expect(screen.queryByRole("button", { name: "打开" })).toBeNull();
  });

  it("surfaces a failed discard instead of closing silently", async () => {
    const user = userEvent.setup();
    const errorToast = vi.spyOn(toast, "error").mockImplementation(() => "id");
    renderDialog(
      [
        historyEntry({
          id: "run-1",
          status: "succeeded",
          worktreeTaskId: "worktree-task-1",
        }),
      ],
      { deletedTaskIds: [], failDiscards: true },
    );

    await user.click(await screen.findByRole("button", { name: "丢弃" }));
    await user.click(await screen.findByRole("button", { name: "丢弃" }));

    await waitFor(() => expect(errorToast).toHaveBeenCalledTimes(1));
  });
});
