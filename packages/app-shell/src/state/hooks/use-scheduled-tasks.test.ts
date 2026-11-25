import { describe, expect, it } from "vitest";
import { waitFor } from "@testing-library/react";
import type { ScheduledTask, ScheduledTaskHistoryEntry } from "@ora/contracts";
import {
  createTestClient,
  type TestHandlers,
} from "../../test/contracts-transport";
import { renderHookWithClient } from "../../test/hook-harness";
import {
  scheduledTaskKeys,
  useCreateScheduledTask,
  useRevokeScheduledTaskApproval,
  useScheduledTaskHistory,
  useScheduledTasks,
} from "./use-scheduled-tasks";

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

const HISTORY: ScheduledTaskHistoryEntry[] = [
  {
    kind: "missed",
    scheduledAt: 20n,
    reason: "late_trigger",
    observedAt: 620_001n,
  },
  {
    kind: "run",
    id: "run-1",
    scheduledAt: 10n,
    revision: 1n,
    status: "succeeded",
    sessionId: "session-1",
    worktreePath: "worktrees/run-1",
    worktreeTaskId: "worktree-task-1",
    failureReason: null,
    verdict: null,
    verdictDetail: null,
    verdictAt: null,
    createdAt: 10n,
    updatedAt: 15n,
  },
];

function scheduledTaskHandlers(tasks: ScheduledTask[]): TestHandlers {
  return {
    listScheduledTasks: () => ({ tasks: [...tasks] }),
    listScheduledTaskHistory: () => ({ entries: HISTORY }),
    createScheduledTask: (request) => {
      const task: ScheduledTask = {
        ...request,
        id: "scheduled-created",
        revision: 1n,
        approval:
          request.approvalPolicy === null
            ? null
            : { policy: request.approvalPolicy, consentedAt: 20n },
        worktreeBaseBranch: "main",
        createdAt: 20n,
        updatedAt: 20n,
      };
      tasks.push(task);
      return { task };
    },
    revokeScheduledTaskApproval: ({ taskId }) => {
      const index = tasks.findIndex((task) => task.id === taskId);
      const task = { ...tasks[index]!, enabled: false, approval: null };
      tasks[index] = task;
      return { task };
    },
  };
}

describe("Scheduled Task hooks", () => {
  it("loads definitions through the generated scheduledTask namespace", async () => {
    const client = createTestClient(scheduledTaskHandlers([TASK]));
    const hook = renderHookWithClient(() => useScheduledTasks(), client);

    await waitFor(() => expect(hook.result.current.isSuccess).toBe(true));
    expect(hook.result.current.data).toEqual([TASK]);
  });

  it("loads newest-first persisted history through the generated namespace", async () => {
    const client = createTestClient(scheduledTaskHandlers([TASK]));
    const hook = renderHookWithClient(
      () => useScheduledTaskHistory(TASK.id),
      client,
    );

    await waitFor(() => expect(hook.result.current.isSuccess).toBe(true));
    expect(hook.result.current.data).toEqual(HISTORY);
  });

  it("refreshes the shared list after create and approval revocation", async () => {
    const tasks = [TASK];
    const client = createTestClient(scheduledTaskHandlers(tasks));
    const list = renderHookWithClient(() => useScheduledTasks(), client);
    const create = renderHookWithClient(
      () => useCreateScheduledTask(),
      client,
      list.queryClient,
    );
    const revoke = renderHookWithClient(
      () => useRevokeScheduledTaskApproval(),
      client,
      list.queryClient,
    );

    await waitFor(() => expect(list.result.current.isSuccess).toBe(true));
    create.result.current.mutate({
      name: "Hourly quality check",
      enabled: true,
      schedule: { kind: "every_hours", interval: 1 },
      executionLocation: "local",
      projectId: "project-1",
      agentRef: "ora-space/codex",
      model: null,
      prompt: "Run quality checks",
      approvalPolicy: "full_auto_in_worktree",
    });
    await waitFor(() => expect(create.result.current.isSuccess).toBe(true));
    await list.queryClient.refetchQueries({ queryKey: scheduledTaskKeys.all });
    expect(
      list.queryClient.getQueryData<ScheduledTask[]>(scheduledTaskKeys.all),
    ).toHaveLength(2);

    revoke.result.current.mutate(tasks[1]!);
    await waitFor(() => expect(revoke.result.current.isSuccess).toBe(true));
    await list.queryClient.refetchQueries({ queryKey: scheduledTaskKeys.all });
    expect(
      list.queryClient.getQueryData<ScheduledTask[]>(
        scheduledTaskKeys.all,
      )?.[1],
    ).toMatchObject({ enabled: false, approval: null });
  });
});
