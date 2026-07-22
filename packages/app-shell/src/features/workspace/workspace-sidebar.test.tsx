import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import type { Project, Session, Task } from "@ora/contracts";
import { createChatStore, type ChatStore, type SessionConversation } from "@ora/chat";
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
  agentCli: "code_agent_cli",
  status: "running",
};

/** Renders the sidebar with the same provider stack AppShell gives it. */
function renderSidebar(state: MockClientState, chatStore?: ChatStore) {
  const client = createMockClient(state);
  const store = chatStore ?? createChatStore(client.session);
  const Wrapper = createHookWrapper(client, createTestQueryClient(), store);
  return {
    ...render(
      <Wrapper>
        <AppI18nProvider>
          <TooltipProvider>
            <WorkspaceSidebar user={USER} onSignOut={() => undefined} />
          </TooltipProvider>
        </AppI18nProvider>
      </Wrapper>,
    ),
    chatStore: store,
  };
}

/** Builds an idle conversation, overriding only the fields a test cares about. */
function conversation(overrides: Partial<SessionConversation> = {}): SessionConversation {
  return {
    turns: [],
    isLoaded: false,
    isLoading: false,
    isResponding: false,
    pendingPermissions: [],
    error: null,
    ...overrides,
  };
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

/**
 * Finds a tree row by its label.
 *
 * A role query rather than a text one: a branch on its way closed is still in
 * the DOM until the animation ends, and this asks what a user can actually
 * reach at that moment.
 */
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

    await waitFor(() => expect(treeRow(SESSION.agentCli)).not.toBeNull());

    await user.click(screen.getByText(TASK.title));

    expect(treeRow(SESSION.agentCli)).toBeNull();
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

  // The Collapsible holds the panel just long enough to animate out, then drops
  // it, so a collapsed branch costs nothing once the close has finished.
  it("unmounts a collapsed branch instead of leaving it hidden in the DOM", async () => {
    const user = userEvent.setup();
    renderSidebar(workspaceWithOneSession());

    await waitFor(() => expect(treeRow(TASK.title)).not.toBeNull());

    await user.click(screen.getByText(PROJECT.name));

    await waitFor(() => expect(screen.queryByText(TASK.title)).toBeNull());
  });

  // Matches the working-indicator aria-label in either shipped locale.
  const workingIndicator = () => screen.queryByLabelText(/运行中|Running/);

  it("shows no working indicator for a session whose process is alive but idle", async () => {
    // SESSION.status is "running" - the process is up - yet no turn is in flight.
    renderSidebar(workspaceWithOneSession());

    await waitFor(() => expect(treeRow(SESSION.agentCli)).not.toBeNull());
    expect(workingIndicator()).toBeNull();
  });

  it("shows the working indicator only while the session is responding", async () => {
    const store = createChatStore(createMockClient(createMockClientState()).session);
    const { chatStore } = renderSidebar(workspaceWithOneSession(), store);
    await waitFor(() => expect(treeRow(SESSION.agentCli)).not.toBeNull());

    act(() => chatStore.setState({
      conversations: { [SESSION.id]: conversation({ isResponding: true }) },
    }));
    await waitFor(() => expect(workingIndicator()).not.toBeNull());

    act(() => chatStore.setState({
      conversations: { [SESSION.id]: conversation({ isResponding: false }) },
    }));
    await waitFor(() => expect(workingIndicator()).toBeNull());
  });
});
