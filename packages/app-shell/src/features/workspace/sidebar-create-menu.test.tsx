import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createChatStore } from "@ora/chat";
import type { WorkspaceGit } from "@ora/contracts";
import { describe, expect, it, vi } from "vitest";
import { AppI18nProvider } from "../../i18n/i18n";
import "../../i18n/i18n-instance";
import { createTestClient } from "../../test/contracts-transport";
import {
  createHookWrapper,
  createTestQueryClient,
} from "../../test/hook-harness";
import { SidebarCreateMenu } from "./sidebar-create-menu";

/** Renders a project row's create menu whose main workspace folder probe reports `git`. */
function renderProjectMenu(git: WorkspaceGit) {
  const listWorkspaces = vi.fn(async () => ({
    workspaces: [
      {
        id: "workspace-p1",
        projectId: "p1",
        kind: "main" as const,
        lifecycle: "active" as const,
        directory: { kind: "present" as const, git },
      },
    ],
  }));
  const client = createTestClient({
    listWorkspaces,
    listWorkflows: async () => ({ workflows: [] }),
  });
  const Wrapper = createHookWrapper(
    client,
    createTestQueryClient(),
    createChatStore(client.session),
  );
  render(
    <Wrapper>
      <AppI18nProvider>
        <SidebarCreateMenu
          projectId="p1"
          workspaceId="workspace-p1"
          scope="project"
          onNewTask={() => undefined}
        />
      </AppI18nProvider>
    </Wrapper>,
  );
  return { listWorkspaces };
}

describe("SidebarCreateMenu worktree task entry", () => {
  it("leaves out the worktree task for a project folder that is not Git", async () => {
    const { listWorkspaces } = renderProjectMenu("notRepository");
    await waitFor(() => expect(listWorkspaces).toHaveBeenCalled());
    const user = userEvent.setup();

    await user.click(screen.getByRole("button", { name: "在此项目中新建" }));

    expect(
      await screen.findByRole("button", { name: "新建任务" }),
    ).toBeInTheDocument();
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: "新建工作树" })).toBeNull(),
    );
  });

  it("keeps the worktree task for a Git project", async () => {
    renderProjectMenu("repository");
    const user = userEvent.setup();

    await user.click(screen.getByRole("button", { name: "在此项目中新建" }));

    expect(
      await screen.findByRole("button", { name: "新建工作树" }),
    ).toBeInTheDocument();
  });
});
