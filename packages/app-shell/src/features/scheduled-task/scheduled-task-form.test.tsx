import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createChatStore } from "@ora/chat";
import type { WorkspaceGit } from "@ora/contracts";
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

const NOT_GIT_REASON =
  "此项目目录未使用 Git。定时任务在独立工作树中运行，只有 Git 仓库项目的任务才能启用。";

/** Opens a new-task form whose only project has a main folder probe reporting `git`. */
function renderForm(git: WorkspaceGit) {
  const listWorkspaces = vi.fn(async () => ({
    workspaces: [
      {
        id: "workspace-project-1",
        projectId: "project-1",
        kind: "main" as const,
        lifecycle: "active" as const,
        directory: { kind: "present" as const, git },
      },
    ],
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
            task={null}
            projects={[{ id: "project-1", name: "Plain folder" }]}
            agents={[{ agentRef: AGENT_REF.codex, label: "Codex", logo: null }]}
            onOpenChange={() => undefined}
          />
        </TooltipProvider>
      </AppI18nProvider>
    </Wrapper>,
  );
  return { listWorkspaces, queryClient };
}

describe("ScheduledTaskForm project Git requirement", () => {
  it("names the reason and refuses enabling a task for a project folder that is not Git", async () => {
    renderForm("notRepository");
    const user = userEvent.setup();

    // A disabled definition is still accepted by the backend: an informational note, no block.
    expect(await screen.findByText(NOT_GIT_REASON)).toBeInTheDocument();
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.getByRole("button", { name: "创建任务" })).toBeEnabled();

    await user.click(screen.getByRole("switch", { name: "创建后立即启用" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(NOT_GIT_REASON);
    expect(screen.getByRole("button", { name: "创建任务" })).toBeDisabled();
  });

  it("keeps enabling available for a Git project", async () => {
    const { queryClient } = renderForm("repository");
    // Only after the list has landed could a wrong `notRepository` take effect.
    await waitFor(() =>
      expect(queryClient.getQueryState(workspaceKeys.workspaces)?.status).toBe(
        "success",
      ),
    );
    const user = userEvent.setup();

    await user.click(screen.getByRole("switch", { name: "创建后立即启用" }));

    expect(screen.getByRole("button", { name: "创建任务" })).toBeEnabled();
    expect(screen.queryByText(NOT_GIT_REASON)).toBeNull();
  });
});
