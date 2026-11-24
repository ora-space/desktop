import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createChatStore } from "@ora/chat";
import type { WorkspaceGit } from "@ora/contracts";
import { afterEach, describe, expect, it, vi } from "vitest";
import { AppI18nProvider } from "../../i18n/i18n";
import "../../i18n/i18n-instance";
import { createTestClient } from "../../test/contracts-transport";
import {
  createHookWrapper,
  createTestQueryClient,
} from "../../test/hook-harness";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import { ComposerContextBar } from "./composer-context-bar";

const CREATE_BRANCH = "创建并检出新分支…";

afterEach(() => {
  act(() => useWorkspaceSelectionStore.getState().clearSelection());
});

/** Renders the context bar with a worktree task selected in a project whose folder reports `git`. */
async function renderWithTaskSelected(git: WorkspaceGit) {
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
    listProjects: async () => ({ projects: [{ id: "p1", name: "Ora" }] }),
    listTasks: async () => ({
      tasks: [
        {
          id: "t1",
          projectId: "p1",
          workspaceId: "workspace-t1",
          title: "Worktree task",
        },
      ],
    }),
  });
  const Wrapper = createHookWrapper(
    client,
    createTestQueryClient(),
    createChatStore(client.session),
  );
  act(() => useWorkspaceSelectionStore.getState().selectTask("t1", "p1"));
  render(
    <Wrapper>
      <AppI18nProvider>
        <ComposerContextBar />
      </AppI18nProvider>
    </Wrapper>,
  );
  await waitFor(() => expect(listWorkspaces).toHaveBeenCalled());
  const user = userEvent.setup();
  await user.click(await screen.findByRole("button", { name: "选择分支" }));
  // The menu lists the task in both cases; wait for it before asserting on the create row.
  expect((await screen.findAllByText("Worktree task")).length).toBeGreaterThan(
    1,
  );
}

describe("ComposerContextBar new worktree entry", () => {
  it("leaves out creating a worktree once the project folder is not Git", async () => {
    await renderWithTaskSelected("notRepository");

    await waitFor(() => expect(screen.queryByText(CREATE_BRANCH)).toBeNull());
  });

  it("keeps creating a worktree for a Git project", async () => {
    await renderWithTaskSelected("repository");

    expect(await screen.findByText(CREATE_BRANCH)).toBeInTheDocument();
  });
});
