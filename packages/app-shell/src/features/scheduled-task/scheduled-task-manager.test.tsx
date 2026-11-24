import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type {
  ScheduledTask,
  ScheduledTaskHistoryEntry,
  UpdateScheduledTaskRequest,
} from "@ora/contracts";
import { createChatStore } from "@ora/chat";
import { TooltipProvider } from "@ora/ui";
import { PlatformProvider } from "../../platform";
import { AppI18nProvider } from "../../i18n/i18n";
import { appI18n } from "../../i18n/i18n-instance";
import {
  createTestClient,
  type TestHandlers,
} from "../../test/contracts-transport";
import {
  createHookWrapper,
  createTestQueryClient,
} from "../../test/hook-harness";
import { createStubPlatform } from "../../test/stub-platform";
import { AGENT_REF, SEEDED_AGENT_REFS } from "../../test/agent-identity";
import { seededAgentPackages } from "../../test/memory/agent-packages";
import { ScheduledTaskManager } from "./scheduled-task-manager";

void appI18n;

const APPROVAL_LABEL = "我了解风险，并允许此任务无人值守运行";

const CONSENTED_TASK: ScheduledTask = {
  id: "scheduled-1",
  name: "Daily review",
  enabled: true,
  schedule: { kind: "daily", hour: 9, minute: 30 },
  executionLocation: "local",
  projectId: "project-1",
  agentRef: AGENT_REF.codex,
  model: null,
  prompt: "Review the current changes",
  revision: 1n,
  approval: { policy: "full_auto_in_worktree", consentedAt: 10n },
  worktreeBaseBranch: "main",
  createdAt: 10n,
  updatedAt: 10n,
};

const UNCONSENTED_TASK: ScheduledTask = {
  ...CONSENTED_TASK,
  id: "scheduled-2",
  name: "Weekly digest",
  enabled: false,
  agentRef: AGENT_REF.claude,
  approval: null,
};

const CONSENTED_DISABLED_TASK: ScheduledTask = {
  ...CONSENTED_TASK,
  id: "scheduled-3",
  name: "Weekly digest",
  enabled: false,
};

/** Captures every update request so assertions can pin the exact consent semantics. */
function scheduledTaskHandlers(
  tasks: ScheduledTask[],
  updates: UpdateScheduledTaskRequest[],
): TestHandlers {
  return {
    listProjects: async () => ({
      projects: [
        { id: "project-1", name: "Ora Desktop" },
        { id: "project-2", name: "Another project" },
      ],
    }),
    listInstalledPlugins: async () => ({ plugins: seededAgentPackages() }),
    getAgentRuntimeStatus: async () => ({
      statuses: SEEDED_AGENT_REFS.map((agentRef) => ({
        agentRef,
        status: "ready" as const,
      })),
    }),
    listScheduledTasks: async () => ({ tasks: [...tasks] }),
    listScheduledTaskHistory: async () => ({
      entries: [] as ScheduledTaskHistoryEntry[],
    }),
    updateScheduledTask: async (request) => {
      updates.push(request);
      const index = tasks.findIndex((task) => task.id === request.taskId);
      const current = tasks[index]!;
      const approval =
        request.approvalChange.kind === "grant"
          ? {
              policy: request.approvalChange.policy,
              consentedAt: 20n,
            }
          : request.approvalChange.kind === "revoke"
            ? null
            : current.approval;
      const task: ScheduledTask = {
        ...current,
        name: request.name,
        enabled: request.enabled,
        schedule: request.schedule,
        projectId: request.projectId,
        agentRef: request.agentRef,
        model: request.model,
        prompt: request.prompt,
        approval,
        updatedAt: 20n,
      };
      tasks[index] = task;
      return { task };
    },
  };
}

/** Renders the manager with the same provider stack the app shell gives it. */
function renderManager(handlers: TestHandlers) {
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
            <ScheduledTaskManager />
          </TooltipProvider>
        </PlatformProvider>
      </AppI18nProvider>
    </Wrapper>,
  );
}

describe("ScheduledTaskManager consent handling", () => {
  it("opens the new-task form with unattended consent unchecked", async () => {
    const user = userEvent.setup();
    renderManager(scheduledTaskHandlers([], []));

    await user.click(
      await screen.findByRole("button", { name: "新建定时任务" }),
    );

    expect(
      await screen.findByRole("switch", { name: APPROVAL_LABEL }),
    ).not.toBeChecked();
  });

  it("saving an edit without touching consent keeps it and never refreshes consentedAt", async () => {
    const user = userEvent.setup();
    const tasks = [CONSENTED_TASK];
    const updates: UpdateScheduledTaskRequest[] = [];
    renderManager(scheduledTaskHandlers(tasks, updates));

    await user.click(
      await screen.findByRole("button", { name: "编辑定时任务" }),
    );
    const approval = await screen.findByRole("switch", {
      name: APPROVAL_LABEL,
    });
    expect(approval).toBeChecked();
    await user.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => expect(updates).toHaveLength(1));
    expect(updates[0]).toEqual({
      taskId: "scheduled-1",
      expectedUpdatedAt: 10n,
      name: "Daily review",
      enabled: true,
      schedule: { kind: "daily", hour: 9, minute: 30 },
      projectId: "project-1",
      agentRef: AGENT_REF.codex,
      model: null,
      prompt: "Review the current changes",
      approvalChange: { kind: "keep" },
    });
  });

  it("unchecking consent with the task disabled revokes it explicitly", async () => {
    const user = userEvent.setup();
    const tasks = [CONSENTED_TASK];
    const updates: UpdateScheduledTaskRequest[] = [];
    renderManager(scheduledTaskHandlers(tasks, updates));

    await user.click(
      await screen.findByRole("button", { name: "编辑定时任务" }),
    );
    // An enabled task cannot lose its consent, so revoking also disables it.
    await user.click(
      await screen.findByRole("switch", { name: APPROVAL_LABEL }),
    );
    await user.click(screen.getByRole("switch", { name: "创建后立即启用" }));
    await user.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => expect(updates).toHaveLength(1));
    expect(updates[0]?.approvalChange).toEqual({ kind: "revoke" });
  });

  it("checking consent in the form sends a fresh explicit grant", async () => {
    const user = userEvent.setup();
    const tasks = [UNCONSENTED_TASK];
    const updates: UpdateScheduledTaskRequest[] = [];
    renderManager(scheduledTaskHandlers(tasks, updates));

    await user.click(
      await screen.findByRole("button", { name: "编辑定时任务" }),
    );
    const approval = await screen.findByRole("switch", {
      name: APPROVAL_LABEL,
    });
    expect(approval).not.toBeChecked();
    await user.click(approval);
    await user.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => expect(updates).toHaveLength(1));
    expect(updates[0]?.approvalChange).toEqual({
      kind: "grant",
      policy: "full_auto_in_worktree",
    });
  });

  it("requires a fresh consent after changing the target project", async () => {
    const user = userEvent.setup();
    const tasks = [CONSENTED_TASK];
    const updates: UpdateScheduledTaskRequest[] = [];
    renderManager(scheduledTaskHandlers(tasks, updates));

    await user.click(
      await screen.findByRole("button", { name: "编辑定时任务" }),
    );
    const approval = await screen.findByRole("switch", {
      name: APPROVAL_LABEL,
    });
    expect(approval).toBeChecked();
    await user.click(screen.getAllByRole("combobox")[0]!);
    await user.click(
      await screen.findByRole("option", { name: "Another project" }),
    );
    expect(approval).not.toBeChecked();

    await user.click(approval);
    await user.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => expect(updates).toHaveLength(1));
    expect(updates[0]).toMatchObject({
      projectId: "project-2",
      approvalChange: {
        kind: "grant",
        policy: "full_auto_in_worktree",
      },
    });
  });

  it("keeps the new-task title outside the scrolling form body", async () => {
    const user = userEvent.setup();
    renderManager(scheduledTaskHandlers([], []));
    await user.click(
      await screen.findByRole("button", { name: "新建定时任务" }),
    );

    const dialog = await screen.findByRole("dialog");
    expect(dialog.className).toContain("overflow-hidden");
    const title = screen.getByRole("heading", { name: "新建定时任务" });
    expect(title.closest("[data-slot=dialog-header]")?.className).toContain(
      "shrink-0",
    );
    expect(
      screen
        .getByRole("button", { name: "创建任务" })
        .closest("[data-slot=dialog-footer]")?.className,
    ).toContain("shrink-0");
  });

  it("filters the project list from the searchable field", async () => {
    const user = userEvent.setup();
    renderManager(scheduledTaskHandlers([], []));
    await user.click(
      await screen.findByRole("button", { name: "新建定时任务" }),
    );

    const project = await screen.findByRole("combobox", { name: "项目" });
    await user.click(project);
    await user.clear(project);
    await user.type(project, "Another");

    expect(
      await screen.findByRole("option", { name: "Another project" }),
    ).toBeTruthy();
    expect(screen.queryByRole("option", { name: "Ora Desktop" })).toBeNull();
  });

  it("the card enable toggle keeps consent instead of echoing a new grant", async () => {
    const user = userEvent.setup();
    const tasks = [CONSENTED_DISABLED_TASK];
    const updates: UpdateScheduledTaskRequest[] = [];
    renderManager(scheduledTaskHandlers(tasks, updates));

    const card = (await screen.findByText("Weekly digest")).closest("article")!;
    await user.click(within(card).getByRole("switch"));

    await waitFor(() => expect(updates).toHaveLength(1));
    expect(updates[0]).toEqual({
      taskId: "scheduled-3",
      expectedUpdatedAt: 10n,
      name: "Weekly digest",
      enabled: true,
      schedule: { kind: "daily", hour: 9, minute: 30 },
      projectId: "project-1",
      agentRef: AGENT_REF.codex,
      model: null,
      prompt: "Review the current changes",
      approvalChange: { kind: "keep" },
    });
  });

  it("the card enable toggle opens the form instead of enabling a task whose folder is not Git", async () => {
    const user = userEvent.setup();
    const tasks = [CONSENTED_DISABLED_TASK];
    const updates: UpdateScheduledTaskRequest[] = [];
    const listWorkspaces = vi.fn(async () => ({
      workspaces: [
        {
          id: "workspace-project-1",
          projectId: "project-1",
          kind: "main" as const,
          lifecycle: "active" as const,
          directory: {
            kind: "present" as const,
            git: "notRepository" as const,
          },
        },
      ],
    }));
    renderManager({
      ...scheduledTaskHandlers(tasks, updates),
      listWorkspaces,
    });
    await waitFor(() => expect(listWorkspaces).toHaveBeenCalled());

    const card = (await screen.findByText("Weekly digest")).closest("article")!;
    // Let the folder answer land before toggling; a missed gate then sends an update at once.
    await act(async () => {
      await listWorkspaces.mock.results[0]?.value;
    });
    await user.click(within(card).getByRole("switch"));

    // The form opens with the task still disabled and explains it cannot be enabled.
    expect(
      await screen.findByText(
        "此项目目录未使用 Git，任务只能保持停用；其他修改仍可保存。",
      ),
    ).toBeInTheDocument();
    expect(updates).toEqual([]);
  });
});
