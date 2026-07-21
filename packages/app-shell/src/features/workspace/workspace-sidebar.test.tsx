import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import type { Project, Session, Task } from "@ora/contracts";
import { createChatStore } from "@ora/chat";
import { TooltipProvider } from "@ora/ui";
import { AppI18nProvider } from "../../i18n/i18n";
import { createMockClient, createMockClientState, type MockClientState } from "../../test/mock-client";
import { createHookWrapper, createTestQueryClient } from "../../test/hook-harness";
import { useUiStore } from "../../state/stores/ui-store";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import { WorkspaceSidebar } from "./workspace-sidebar";

const USER = { name: "Eric", email: "eric@example.com" };
// Deliberately not "Ora": the sidebar header renders that as the product mark,
// so a project of the same name makes every text query ambiguous.
const PROJECT: Project = { id: "p1", name: "Ora Desktop", rootPath: "/ora" };
const TASK: Task = { id: "t1", projectId: "p1", title: "Refactor", status: "todo" };
const SESSION: Session = {
  id: "s1",
  taskId: "t1",
  agentId: "codex",
  agentSessionId: "agent-1",
  status: "running",
};

/** Renders the sidebar with the same provider stack AppShell gives it. */
function renderSidebar(state: MockClientState) {
  const Wrapper = createHookWrapper(
    createMockClient(state),
    createTestQueryClient(),
    createChatStore({
      newSession: async () => ({ sessionId: "agent-session-test" }),
      prompt: async () => ({ stopReason: "end_turn" }),
      subscribe: () => () => undefined,
    }),
  );
  return render(
    <Wrapper>
      <AppI18nProvider>
        <TooltipProvider>
          <WorkspaceSidebar user={USER} onSignOut={() => undefined} />
        </TooltipProvider>
      </AppI18nProvider>
    </Wrapper>,
  );
}

/** Populates the tree the collapse tests operate on. */
function workspaceWithOneSession(): MockClientState {
  const state = createMockClientState();
  state.projects = [PROJECT];
  state.tasks = [TASK];
  state.sessions = [SESSION];
  return state;
}

beforeEach(() => {
  useWorkspaceSelectionStore.getState().clearSelection();
  useUiStore.setState({ expandedProjects: new Set(), expandedTasks: new Set() });
});

/** Finds a tree row by its label. */
function treeRow(label: string): HTMLElement | null {
  return screen.queryByRole("button", { name: new RegExp(label) });
}

describe("WorkspaceSidebar", () => {
  // Regression: selecting a row used to re-expand its ancestors, so the first
  // click on an expanded row selected and silently re-opened it, and only the
  // second click appeared to collapse anything.
  it("collapses a project on the first click, not the second", async () => {
    const user = userEvent.setup();
    renderSidebar(workspaceWithOneSession());

    await waitFor(() => expect(treeRow(TASK.title)).not.toBeNull());

    await user.click(screen.getByText(PROJECT.name));

    expect(treeRow(TASK.title)).toBeNull();
    expect(useUiStore.getState().expandedProjects.has(PROJECT.id)).toBe(false);
  });

  it("collapses a task on the first click, not the second", async () => {
    const user = userEvent.setup();
    renderSidebar(workspaceWithOneSession());

    await waitFor(() => expect(treeRow(SESSION.agentId)).not.toBeNull());

    await user.click(screen.getByText(TASK.title));

    expect(treeRow(SESSION.agentId)).toBeNull();
    expect(useUiStore.getState().expandedTasks.has(TASK.id)).toBe(false);
  });

  it("re-expands a collapsed project on the next click", async () => {
    const user = userEvent.setup();
    renderSidebar(workspaceWithOneSession());

    await waitFor(() => expect(treeRow(TASK.title)).not.toBeNull());

    await user.click(screen.getByText(PROJECT.name));
    await user.click(screen.getByText(PROJECT.name));

    expect(treeRow(TASK.title)).not.toBeNull();
  });
});
