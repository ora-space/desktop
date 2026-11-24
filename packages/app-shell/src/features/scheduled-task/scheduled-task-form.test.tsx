import { render, screen, waitFor } from "@testing-library/react";
import type { QueryClient } from "@tanstack/react-query";
import userEvent from "@testing-library/user-event";
import { createChatStore } from "@ora/chat";
import type { ScheduledTask, WorkspaceGit } from "@ora/contracts";
import { TooltipProvider } from "@ora/ui";
import { describe, expect, it, vi } from "vitest";
import { AppI18nProvider } from "../../i18n/i18n";
import { appI18n } from "../../i18n/i18n-instance";
import { createTestClient } from "../../test/contracts-transport";
import {
  createHookWrapper,
  createTestQueryClient,
} from "../../test/hook-harness";
import { AGENT_REF } from "../../test/agent-identity";
import { workspaceKeys } from "../../state/data/workspace";
import { ScheduledTaskForm } from "./scheduled-task-form";

void appI18n;

const CREATE_REFUSAL =
  "此项目目录未使用 Git，无法为它创建定时任务。定时任务在独立工作树中运行，只能用于 Git 仓库项目。";
const SWITCH_REFUSAL =
  "此项目目录未使用 Git，不能把任务改到这个项目。定时任务在独立工作树中运行，只能用于 Git 仓库项目。";
const ENABLE_REFUSAL =
  "此项目目录未使用 Git，任务无法启用。关闭“创建后立即启用”后仍可保存其他修改。";
const DISABLED_NOTE =
  "此项目目录未使用 Git，任务只能保持停用；其他修改仍可保存。";

/** A disabled, approved task on project-1. */
const DISABLED_TASK: ScheduledTask = {
  id: "scheduled-1",
  name: "Weekly digest",
  enabled: false,
  schedule: { kind: "daily", hour: 9, minute: 30 },
  executionLocation: "local",
  projectId: "project-1",
  agentRef: AGENT_REF.codex,
  model: null,
  prompt: "Review the current changes",
  revision: 1n,
  approval: { policy: "full_auto_in_worktree", consentedAt: 10n },
  createdAt: 10n,
  updatedAt: 10n,
};

/**
 * Opens the form for `task` (or a new task) with project-1 and project-2, whose main folder probes
 * report the given capabilities.
 */
function renderForm(
  git: { project1: WorkspaceGit; project2?: WorkspaceGit },
  task: ScheduledTask | null = null,
) {
  const listWorkspaces = vi.fn(async () => ({
    workspaces: (
      [
        ["project-1", git.project1],
        ["project-2", git.project2 ?? "repository"],
      ] as const
    ).map(([projectId, probe]) => ({
      id: `workspace-${projectId}`,
      projectId,
      kind: "main" as const,
      lifecycle: "active" as const,
      directory: { kind: "present" as const, git: probe },
    })),
  }));
  const client = createTestClient({ listWorkspaces });
  const queryClient = createTestQueryClient();
  const Wrapper = createHookWrapper(
    client,
    queryClient,
    createChatStore(client.session),
  );
  render(
    <Wrapper>
      <AppI18nProvider>
        <TooltipProvider>
          <ScheduledTaskForm
            open
            task={task}
            projects={[
              { id: "project-1", name: "First project" },
              { id: "project-2", name: "Second project" },
            ]}
            agents={[{ agentRef: AGENT_REF.codex, label: "Codex", logo: null }]}
            onOpenChange={() => undefined}
          />
        </TooltipProvider>
      </AppI18nProvider>
    </Wrapper>,
  );
  return { queryClient };
}

/** Waits until the workspace list has landed, after which a wrong answer would show. */
async function waitForWorkspaceList(queryClient: QueryClient) {
  await waitFor(() =>
    expect(queryClient.getQueryState(workspaceKeys.workspaces)?.status).toBe(
      "success",
    ),
  );
}

describe("ScheduledTaskForm project Git requirement", () => {
  it("blocks creating a task for a project folder that is not Git", async () => {
    renderForm({ project1: "notRepository" });

    expect(await screen.findByRole("alert")).toHaveTextContent(CREATE_REFUSAL);
    // Creation resolves the project's branch even for a disabled task, so no save is offered.
    expect(
      screen.getByRole("switch", { name: "创建后立即启用" }),
    ).not.toBeChecked();
    expect(screen.getByRole("button", { name: "创建任务" })).toBeDisabled();
  });

  it("keeps creating and enabling available for a Git project", async () => {
    const { queryClient } = renderForm({ project1: "repository" });
    await waitForWorkspaceList(queryClient);
    const user = userEvent.setup();

    await user.click(screen.getByRole("switch", { name: "创建后立即启用" }));

    expect(screen.getByRole("button", { name: "创建任务" })).toBeEnabled();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("lets a disabled task on a plain folder be edited, but not enabled", async () => {
    const { queryClient } = renderForm(
      { project1: "notRepository" },
      DISABLED_TASK,
    );
    await waitForWorkspaceList(queryClient);
    const user = userEvent.setup();

    expect(await screen.findByText(DISABLED_NOTE)).toBeInTheDocument();
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.getByRole("button", { name: "保存" })).toBeEnabled();

    await user.click(screen.getByRole("switch", { name: "创建后立即启用" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(ENABLE_REFUSAL);
    expect(screen.getByRole("button", { name: "保存" })).toBeDisabled();
  });

  it("blocks moving a task to a project folder that is not Git", async () => {
    const { queryClient } = renderForm(
      { project1: "repository", project2: "notRepository" },
      DISABLED_TASK,
    );
    await waitForWorkspaceList(queryClient);
    const user = userEvent.setup();
    expect(screen.getByRole("button", { name: "保存" })).toBeEnabled();

    const project = screen.getByRole("combobox", { name: "项目" });
    await user.click(project);
    await user.clear(project);
    await user.type(project, "Second");
    await user.click(
      await screen.findByRole("option", { name: "Second project" }),
    );

    expect(await screen.findByRole("alert")).toHaveTextContent(SWITCH_REFUSAL);
    expect(screen.getByRole("button", { name: "保存" })).toBeDisabled();
  });
});
