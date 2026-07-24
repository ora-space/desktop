import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import type { Project, Session, Task } from "@ora/contracts";
import { createChatStore, type ChatStore, type SessionConversation } from "@ora/chat";
import { TooltipProvider } from "@ora/ui";
import { PlatformProvider } from "@ora/platform";
import { AppI18nProvider } from "../../i18n/i18n";
import { createMockClient, createMockClientState, type MockClientState } from "../../test/mock-client";
import { createHookWrapper, createTestQueryClient } from "../../test/hook-harness";
import { createStubPlatform } from "../../test/stub-platform";
import { useUiStore } from "../../state/stores/ui-store";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import { useUnreadSessionsStore } from "../../state/stores/unread-sessions-store";
import { WorkspaceSidebar } from "./workspace-sidebar";

const USER = { name: "Eric", email: "eric@example.com" };
// Deliberately not "Ora": the sidebar header renders that as the product mark,
// so a project of the same name makes every text query ambiguous.
const PROJECT: Project = { id: "p1", name: "Ora Desktop", rootPath: "/ora" };
const TASK: Task = { id: "t1", projectId: "p1", title: "Refactor", status: "todo", workspaceMode: "worktree" };
const SESSION: Session = {
  id: "s1",
  taskId: "t1",
  agentCli: "open_code",
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
          <PlatformProvider adapter={createStubPlatform()}>
            <TooltipProvider>
              <WorkspaceSidebar user={USER} onSignOut={() => undefined} />
            </TooltipProvider>
          </PlatformProvider>
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
    availableCommands: [],
    modes: null,
    sessionTitle: null,
    sessionUpdatedAt: null,
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
  useUiStore.setState({
    expandedProjects: new Set(),
    expandedTasks: new Set(),
    dialog: null,
    deleteTarget: null,
  });
  useUnreadSessionsStore.setState({ unread: new Set() });
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
  it("only toggles project expansion when the project row is clicked", async () => {
    const user = userEvent.setup();
    useWorkspaceSelectionStore.getState().selectSession(SESSION.id, TASK.id, PROJECT.id);
    renderSidebar(workspaceWithOneSession());

    await waitFor(() => expect(treeRow(TASK.title)).not.toBeNull());
    await user.click(screen.getByText(PROJECT.name));

    expect(useWorkspaceSelectionStore.getState().selection).toEqual({
      projectId: PROJECT.id,
      taskId: TASK.id,
      sessionId: SESSION.id,
    });
    expect(useUiStore.getState().expandedProjects.has(PROJECT.id)).toBe(false);
  });

  it("opens worktree creation from the project branch action", async () => {
    const user = userEvent.setup();
    renderSidebar(workspaceWithOneSession());

    await user.click(await screen.findByRole("button", { name: /新建工作树任务|New worktree task/ }));

    expect(useUiStore.getState().dialog).toEqual({
      kind: "task",
      projectId: PROJECT.id,
    });
    expect(useWorkspaceSelectionStore.getState().selection).toEqual({
      projectId: null,
      taskId: null,
      sessionId: null,
    });
  });

  it("starts a blank direct chat from the project edit action", async () => {
    const user = userEvent.setup();
    useWorkspaceSelectionStore.getState().selectSession(SESSION.id, TASK.id, PROJECT.id);
    renderSidebar(workspaceWithOneSession());

    await user.click(await screen.findByRole("button", { name: /新建任务|New task/ }));

    expect(useWorkspaceSelectionStore.getState().selection).toEqual({
      projectId: PROJECT.id,
      taskId: null,
      sessionId: null,
    });
    expect(useUiStore.getState().dialog).toBeNull();
  });

  it("collects every descendant session when deleting a project", async () => {
    const user = userEvent.setup();
    const state = workspaceWithOneSession();
    state.tasks.push({
      id: "t2",
      projectId: PROJECT.id,
      title: "Direct chat",
      status: "todo",
      workspaceMode: "project_root",
    });
    state.sessions.push({
      id: "s2",
      taskId: "t2",
      agentCli: "open_code",
      status: "running",
    });
    renderSidebar(state);

    const projectRow = (await screen.findByRole("button", { name: new RegExp(PROJECT.name) })).parentElement!;
    await user.click(within(projectRow).getByRole("button", { name: /打开操作菜单|Open actions/ }));
    await user.click(await screen.findByRole("menuitem", { name: /删除|Delete/ }));

    expect(useUiStore.getState().deleteTarget).toEqual({
      kind: "project",
      id: PROJECT.id,
      name: PROJECT.name,
      sessionIds: ["s1", "s2"],
    });
  });

  it("starts a new direct chat without losing the selected project", async () => {
    const user = userEvent.setup();
    useWorkspaceSelectionStore.getState().selectSession(SESSION.id, TASK.id, PROJECT.id);
    renderSidebar(workspaceWithOneSession());

    await user.click(await screen.findByRole("button", { name: /新建对话|New chat/ }));

    expect(useWorkspaceSelectionStore.getState().selection).toEqual({
      projectId: PROJECT.id,
      taskId: null,
      sessionId: null,
    });
  });

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

    await waitFor(() => expect(treeRow("OpenCode")).not.toBeNull());

    await user.click(screen.getByText(TASK.title));

    expect(treeRow("OpenCode")).toBeNull();
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

    await waitFor(() => expect(treeRow("OpenCode")).not.toBeNull());
    expect(workingIndicator()).toBeNull();
  });

  it("uses a message icon for direct-chat tasks and a branch icon for worktrees", async () => {
    const state = createMockClientState();
    state.projects = [PROJECT];
    state.tasks = [
      TASK,
      { id: "t2", projectId: PROJECT.id, title: "Direct chat", status: "todo", workspaceMode: "project_root" },
    ];
    renderSidebar(state);

    await waitFor(() => expect(treeRow("Direct chat")).not.toBeNull());
    expect(screen.getByLabelText(/直聊任务|Direct chat task/)).not.toBeNull();
    expect(screen.getByLabelText(/Git 工作树任务|Git worktree task/)).not.toBeNull();
  });

  it("uses the agent-provided session title as the session row label", async () => {
    const store = createChatStore(createMockClient(createMockClientState()).session);
    const { chatStore } = renderSidebar(workspaceWithOneSession(), store);
    await waitFor(() => expect(treeRow("OpenCode")).not.toBeNull());

    act(() => chatStore.setState({
      conversations: { [SESSION.id]: conversation({ sessionTitle: "Review auth flow" }) },
    }));

    await waitFor(() => expect(treeRow("Review auth flow")).not.toBeNull());
    expect(treeRow("OpenCode")).toBeNull();
  });

  it("shows the working indicator only while the session is responding", async () => {
    const store = createChatStore(createMockClient(createMockClientState()).session);
    const { chatStore } = renderSidebar(workspaceWithOneSession(), store);
    await waitFor(() => expect(treeRow("OpenCode")).not.toBeNull());

    act(() => chatStore.setState({
      conversations: { [SESSION.id]: conversation({ isResponding: true }) },
    }));
    await waitFor(() => expect(workingIndicator()).not.toBeNull());

    act(() => chatStore.setState({
      conversations: { [SESSION.id]: conversation({ isResponding: false }) },
    }));
    await waitFor(() => expect(workingIndicator()).toBeNull());
  });

  // Matches the unread-mark aria-label in either shipped locale.
  const unreadMark = () => screen.queryByLabelText(/有未读更新|Unread/);

  it("shows an unread mark for an idle session flagged unread", async () => {
    useUnreadSessionsStore.setState({ unread: new Set([SESSION.id]) });
    renderSidebar(workspaceWithOneSession());

    await waitFor(() => expect(treeRow("OpenCode")).not.toBeNull());
    expect(unreadMark()).not.toBeNull();
    // The working animation is a distinct, higher-priority state.
    expect(workingIndicator()).toBeNull();
  });

  it("prefers the working animation over the unread mark while responding", async () => {
    useUnreadSessionsStore.setState({ unread: new Set([SESSION.id]) });
    const store = createChatStore(createMockClient(createMockClientState()).session);
    const { chatStore } = renderSidebar(workspaceWithOneSession(), store);
    await waitFor(() => expect(treeRow("OpenCode")).not.toBeNull());

    act(() => chatStore.setState({
      conversations: { [SESSION.id]: conversation({ isResponding: true }) },
    }));

    await waitFor(() => expect(workingIndicator()).not.toBeNull());
    expect(unreadMark()).toBeNull();
  });
});
