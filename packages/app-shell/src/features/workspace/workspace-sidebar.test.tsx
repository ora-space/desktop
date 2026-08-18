import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  LocalTransportError,
  type Project,
  type Session,
  type Task,
} from "@ora/contracts";
import {
  createChatStore,
  type ChatStore,
  type SessionConversation,
} from "@ora/chat";
import { TooltipProvider } from "@ora/ui";
import { PlatformProvider } from "../../platform";
import { AppI18nProvider } from "../../i18n/i18n";
import {
  createMockClient,
  createMockClientState,
  type MockClientState,
  type MockWorkflowRecord,
} from "../../test/mock-client";
import {
  createHookWrapper,
  createTestQueryClient,
} from "../../test/hook-harness";
import { createStubPlatform } from "../../test/stub-platform";
import { useUiStore } from "../../state/stores/ui-store";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import { useUnreadSessionsStore } from "../../state/stores/unread-sessions-store";
import { WorkspaceSidebar } from "./workspace-sidebar";

const USER = { name: "Eric", email: "eric@example.com" };
// Deliberately not "Ora": the sidebar header renders that as the product mark,
// so a project of the same name makes every text query ambiguous.
const PROJECT: Project = { id: "p1", name: "Ora Desktop", rootPath: "/ora" };
const TASK: Task = {
  id: "t1",
  projectId: "p1",
  title: "Refactor",
  workspaceMode: "worktree",
  type: "default",
  workflowRunId: null,
};
const SESSION: Session = {
  id: "s1",
  taskId: "t1",
  agentCli: "open_code",
  status: "running",
  title: null,
  historyState: { type: "writable" },
};

/** Renders the sidebar with the same provider stack AppShell gives it. */
function renderSidebar(
  state: MockClientState,
  chatStore?: ChatStore,
  client = createMockClient(state),
) {
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
function conversation(
  overrides: Partial<SessionConversation> = {},
): SessionConversation {
  return {
    configOptions: [],
    modelChanges: [],
    historyNotices: [],
    turns: [],
    availableCommands: [],
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

/** A library workflow with a published snapshot — the only kind the sidebar picker lists. */
function mockPublishedWorkflow(id: string, name: string): MockWorkflowRecord {
  return {
    workflow: {
      id,
      namespace: "local",
      name,
      publishedSnapshotId: `${id}-pub`,
      createdAt: 0n,
      updatedAt: 0n,
    },
    draft: {
      id: `${id}-draft`,
      workflowId: id,
      version: "draft",
      graph: "{}",
      createdAt: 0n,
      updatedAt: null,
    },
    published: [
      {
        id: `${id}-pub`,
        workflowId: id,
        version: "v1",
        graph: "{}",
        createdAt: 0n,
        updatedAt: null,
      },
    ],
  };
}

/** A draft-only workflow. Sidebar create must not offer it. */
function mockDraftWorkflow(id: string, name: string): MockWorkflowRecord {
  return {
    workflow: {
      id,
      namespace: "local",
      name,
      publishedSnapshotId: null,
      createdAt: 0n,
      updatedAt: 0n,
    },
    draft: {
      id: `${id}-draft`,
      workflowId: id,
      version: "draft",
      graph: "{}",
      createdAt: 0n,
      updatedAt: null,
    },
    published: [],
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
  document.body.removeAttribute("style");
});

/**
 * Finds a tree row by its label.
 *
 * A role query rather than a text one: a branch on its way closed is still in
 * the DOM until the animation ends, and this asks what a user can actually
 * reach at that moment.
 */
function treeRow(label: string): HTMLElement | null {
  return (
    screen
      .queryAllByRole("button", { name: new RegExp(label) })
      .find((element) => element.classList.contains("h-full")) ?? null
  );
}

const NEW_SESSION_LABEL = "新建会话|New session";

describe("WorkspaceSidebar", () => {
  it("only toggles project expansion when the project row is clicked", async () => {
    const user = userEvent.setup();
    useWorkspaceSelectionStore
      .getState()
      .selectSession(SESSION.id, TASK.id, PROJECT.id);
    renderSidebar(workspaceWithOneSession());

    await waitFor(() => expect(treeRow(TASK.title)).not.toBeNull());
    await user.click(screen.getByText(PROJECT.name));

    expect(useWorkspaceSelectionStore.getState().selection).toEqual({
      projectId: PROJECT.id,
      taskId: TASK.id,
      sessionId: SESSION.id,
      workflowRunId: null,
    });
    expect(useUiStore.getState().expandedProjects.has(PROJECT.id)).toBe(false);
  });

  it("opens worktree creation from the create menu", async () => {
    const user = userEvent.setup();
    renderSidebar(workspaceWithOneSession());

    await waitFor(() => expect(treeRow(PROJECT.name)).not.toBeNull());
    await user.click(
      await screen.findByRole("button", {
        name: /在此项目中新建|Create in this project/,
      }),
    );
    await user.click(
      await screen.findByRole("button", {
        name: /新建工作树任务|New worktree task/,
      }),
    );

    expect(useUiStore.getState().dialog).toEqual({
      kind: "task",
      projectId: PROJECT.id,
    });
  });

  it("starts a blank chat from the new-chat control above search", async () => {
    const user = userEvent.setup();
    useWorkspaceSelectionStore
      .getState()
      .selectSession(SESSION.id, TASK.id, PROJECT.id);
    renderSidebar(workspaceWithOneSession());

    await user.click(
      await screen.findByRole("button", { name: /新建对话|New chat/ }),
    );

    expect(useWorkspaceSelectionStore.getState().selection).toEqual({
      projectId: PROJECT.id,
      taskId: null,
      sessionId: null,
      workflowRunId: null,
    });
  });

  it("starts a blank direct chat from the create menu", async () => {
    const user = userEvent.setup();
    useWorkspaceSelectionStore
      .getState()
      .selectSession(SESSION.id, TASK.id, PROJECT.id);
    renderSidebar(workspaceWithOneSession());

    await waitFor(() => expect(treeRow(PROJECT.name)).not.toBeNull());
    await user.click(
      await screen.findByRole("button", {
        name: /在此项目中新建|Create in this project/,
      }),
    );
    await user.click(
      await screen.findByRole("button", { name: /^新建任务$|^New task$/ }),
    );

    expect(useWorkspaceSelectionStore.getState().selection).toEqual({
      projectId: PROJECT.id,
      taskId: null,
      sessionId: null,
      workflowRunId: null,
    });
    expect(useUiStore.getState().dialog).toBeNull();
  });

  it("creates a worktree task from the project row plus", async () => {
    const user = userEvent.setup();
    renderSidebar(workspaceWithOneSession());

    await waitFor(() => expect(treeRow(PROJECT.name)).not.toBeNull());
    await user.click(
      await screen.findByRole("button", {
        name: /在此项目中新建|Create in this project/,
      }),
    );
    await user.click(
      await screen.findByRole("button", {
        name: /新建工作树任务|New worktree task/,
      }),
    );

    expect(useUiStore.getState().dialog).toEqual({
      kind: "task",
      projectId: PROJECT.id,
    });
  });

  it("collects every descendant session when deleting a project", async () => {
    const user = userEvent.setup();
    const state = workspaceWithOneSession();
    state.tasks.push({
      id: "t2",
      projectId: PROJECT.id,
      title: "Direct chat",
      workspaceMode: "project_root",
      type: "default",
      workflowRunId: null,
    });
    state.sessions.push({
      id: "s2",
      taskId: "t2",
      agentCli: "open_code",
      status: "running",
      title: null,
      historyState: { type: "writable" },
    });
    renderSidebar(state);

    await waitFor(() => expect(treeRow(PROJECT.name)).not.toBeNull());
    await user.pointer({
      keys: "[MouseRight>]",
      target: treeRow(PROJECT.name)!,
    });
    await user.click(
      await screen.findByRole("menuitem", { name: /删除|Delete/ }),
    );

    expect(useUiStore.getState().deleteTarget).toEqual({
      kind: "project",
      id: PROJECT.id,
      name: PROJECT.name,
      sessionIds: ["s1", "s2"],
    });
  });

  it("does not render overflow menus on workspace tree rows", async () => {
    renderSidebar(workspaceWithOneSession());

    await waitFor(() => expect(treeRow(PROJECT.name)).not.toBeNull());
    expect(
      screen.queryByRole("button", { name: /打开操作菜单|Open actions/ }),
    ).toBeNull();
  });

  it("starts a new direct chat from the create menu without losing the selected project", async () => {
    const user = userEvent.setup();
    useWorkspaceSelectionStore
      .getState()
      .selectSession(SESSION.id, TASK.id, PROJECT.id);
    renderSidebar(workspaceWithOneSession());

    await waitFor(() => expect(treeRow(PROJECT.name)).not.toBeNull());
    await user.click(
      await screen.findByRole("button", {
        name: /在此项目中新建|Create in this project/,
      }),
    );
    await user.click(
      await screen.findByRole("button", { name: /^新建任务$|^New task$/ }),
    );

    expect(useWorkspaceSelectionStore.getState().selection).toEqual({
      projectId: PROJECT.id,
      taskId: null,
      sessionId: null,
      workflowRunId: null,
    });
  });

  it("shows an archive control on session rows instead of the overflow menu", async () => {
    renderSidebar(workspaceWithOneSession());

    await waitFor(() => expect(treeRow(NEW_SESSION_LABEL)).not.toBeNull());
    const sessionRow = treeRow(NEW_SESSION_LABEL)!.parentElement!;
    expect(
      within(sessionRow).getByRole("button", { name: /归档|Archive/ }),
    ).not.toBeNull();
    expect(
      within(sessionRow).queryByRole("button", {
        name: /打开操作菜单|Open actions/,
      }),
    ).toBeNull();
  });

  it("shows an archive control on workflow run rows", async () => {
    const state = workspaceWithOneSession();
    state.workflowRuns = [
      {
        id: "run1",
        projectId: PROJECT.id,
        workflowId: "wf1",
        snapshotId: "snap1",
        name: "Review bot",
        status: "pending",
        taskId: "wt1",
        createdAt: 0n,
        updatedAt: 0n,
      },
    ];
    renderSidebar(state);

    await waitFor(() => expect(treeRow("Review bot")).not.toBeNull());
    const runRow = treeRow("Review bot")!.closest(
      "div.group\\/tree",
    ) as HTMLElement;
    expect(
      within(runRow).getByRole("button", { name: /归档|Archive/ }),
    ).not.toBeNull();
  });

  it("opens delete confirmation from the session context menu", async () => {
    const user = userEvent.setup();
    renderSidebar(workspaceWithOneSession());

    await waitFor(() => expect(treeRow(NEW_SESSION_LABEL)).not.toBeNull());
    await user.pointer({
      keys: "[MouseRight>]",
      target: treeRow(NEW_SESSION_LABEL)!,
    });
    await user.click(
      await screen.findByRole("menuitem", { name: /删除|Delete/ }),
    );

    expect(useUiStore.getState().deleteTarget).toEqual({
      kind: "session",
      id: SESSION.id,
      name: "新建会话",
    });
  });

  it("opens deploy dialog state when a workflow template is chosen", async () => {
    const user = userEvent.setup({ pointerEventsCheck: 0 });
    const state = workspaceWithOneSession();
    state.workflows = [
      mockPublishedWorkflow("wf1", "Deploy bot"),
      mockDraftWorkflow("wf2", "Unpublished draft"),
    ];
    renderSidebar(state);

    await waitFor(() => expect(treeRow(PROJECT.name)).not.toBeNull());
    await user.click(
      await screen.findByRole("button", {
        name: /在此项目中新建|Create in this project/,
      }),
    );
    await user.hover(
      await screen.findByRole("button", {
        name: /新建工作流任务|New workflow task/,
      }),
    );
    await user.click(
      await screen.findByRole("button", {
        name: /新建工作流任务|New workflow task/,
      }),
    );
    expect(
      screen.queryByRole("button", { name: "Unpublished draft" }),
    ).toBeNull();
    await user.click(await screen.findByRole("button", { name: "Deploy bot" }));

    expect(useUiStore.getState().dialog).toEqual({
      kind: "deployWorkflow",
      projectId: PROJECT.id,
      workflowId: "wf1",
      workflowName: "Deploy bot",
    });
  });

  it("moves focus into the workflow search when opened from the keyboard", async () => {
    const user = userEvent.setup({ pointerEventsCheck: 0 });
    const state = workspaceWithOneSession();
    state.workflows = [mockPublishedWorkflow("wf1", "Deploy bot")];
    renderSidebar(state);

    await waitFor(() => expect(treeRow(PROJECT.name)).not.toBeNull());
    await user.click(
      await screen.findByRole("button", {
        name: /在此项目中新建|Create in this project/,
      }),
    );
    const workflowButton = await screen.findByRole("button", {
      name: /新建工作流任务|New workflow task/,
    });
    workflowButton.focus();
    await user.keyboard("{Enter}");

    expect(
      await screen.findByRole("textbox", {
        name: /搜索工作流模板|Search workflow templates/,
      }),
    ).toHaveFocus();
  });

  it("renames a session from the context menu", async () => {
    const user = userEvent.setup();
    const state = workspaceWithOneSession();
    renderSidebar(state);

    await waitFor(() => expect(treeRow(NEW_SESSION_LABEL)).not.toBeNull());
    await user.pointer({
      keys: "[MouseRight>]",
      target: treeRow(NEW_SESSION_LABEL)!,
    });
    await user.click(
      await screen.findByRole("menuitem", { name: /重命名|Rename/ }),
    );
    const input = await screen.findByRole("textbox", {
      name: /重命名|Rename/,
    });
    fireEvent.change(input, { target: { value: "Review auth" } });
    fireEvent.keyDown(input, { key: "Enter" });

    await waitFor(() => expect(state.sessions[0]?.title).toBe("Review auth"));
    await waitFor(() => expect(treeRow("Review auth")).not.toBeNull());
  });

  it("keeps an in-progress rename draft if Rename is chosen again", async () => {
    const user = userEvent.setup();
    const state = workspaceWithOneSession();
    renderSidebar(state);

    await waitFor(() => expect(treeRow(NEW_SESSION_LABEL)).not.toBeNull());
    await user.pointer({
      keys: "[MouseRight>]",
      target: treeRow(NEW_SESSION_LABEL)!,
    });
    await user.click(
      await screen.findByRole("menuitem", { name: /重命名|Rename/ }),
    );
    const input = await screen.findByRole("textbox", {
      name: /重命名|Rename/,
    });
    fireEvent.change(input, { target: { value: "In progress" } });
    await user.pointer({
      keys: "[MouseRight>]",
      target: input,
    });
    await user.click(
      await screen.findByRole("menuitem", { name: /重命名|Rename/ }),
    );

    expect(
      await screen.findByRole("textbox", { name: /重命名|Rename/ }),
    ).toHaveValue("In progress");
  });

  it("does not commit a session rename while IME is composing", async () => {
    const user = userEvent.setup();
    const state = workspaceWithOneSession();
    renderSidebar(state);

    await waitFor(() => expect(treeRow(NEW_SESSION_LABEL)).not.toBeNull());
    await user.pointer({
      keys: "[MouseRight>]",
      target: treeRow(NEW_SESSION_LABEL)!,
    });
    await user.click(
      await screen.findByRole("menuitem", { name: /重命名|Rename/ }),
    );
    const input = await screen.findByRole("textbox", {
      name: /重命名|Rename/,
    });
    fireEvent.change(input, { target: { value: "zhong" } });
    fireEvent.keyDown(input, { key: "Enter", isComposing: true });

    expect(state.sessions[0]?.title).toBeNull();
    expect(
      await screen.findByRole("textbox", { name: /重命名|Rename/ }),
    ).not.toBeNull();
  });

  it("does not persist twice when blur follows a successful Enter", async () => {
    const user = userEvent.setup();
    const state = workspaceWithOneSession();
    const client = createMockClient(state);
    const rename = vi.fn(client.session.rename);
    client.session.rename = rename;
    renderSidebar(state, undefined, client);

    await waitFor(() => expect(treeRow(NEW_SESSION_LABEL)).not.toBeNull());
    await user.pointer({
      keys: "[MouseRight>]",
      target: treeRow(NEW_SESSION_LABEL)!,
    });
    await user.click(
      await screen.findByRole("menuitem", { name: /重命名|Rename/ }),
    );
    const input = await screen.findByRole("textbox", {
      name: /重命名|Rename/,
    });
    fireEvent.change(input, { target: { value: "Review auth" } });
    fireEvent.keyDown(input, { key: "Enter" });
    fireEvent.blur(input);

    await waitFor(() => expect(state.sessions[0]?.title).toBe("Review auth"));
    expect(rename).toHaveBeenCalledTimes(1);
  });

  it("cancels session rename on Escape without persisting", async () => {
    const user = userEvent.setup();
    const state = workspaceWithOneSession();
    renderSidebar(state);

    await waitFor(() => expect(treeRow(NEW_SESSION_LABEL)).not.toBeNull());
    await user.pointer({
      keys: "[MouseRight>]",
      target: treeRow(NEW_SESSION_LABEL)!,
    });
    await user.click(
      await screen.findByRole("menuitem", { name: /重命名|Rename/ }),
    );
    const input = await screen.findByRole("textbox", {
      name: /重命名|Rename/,
    });
    fireEvent.change(input, { target: { value: "Nope" } });
    fireEvent.keyDown(input, { key: "Escape" });

    await waitFor(() => expect(treeRow(NEW_SESSION_LABEL)).not.toBeNull());
    expect(state.sessions[0]?.title).toBeNull();
  });

  it("rejects an overlong session title without persisting", async () => {
    const user = userEvent.setup();
    const state = workspaceWithOneSession();
    renderSidebar(state);

    await waitFor(() => expect(treeRow(NEW_SESSION_LABEL)).not.toBeNull());
    await user.pointer({
      keys: "[MouseRight>]",
      target: treeRow(NEW_SESSION_LABEL)!,
    });
    await user.click(
      await screen.findByRole("menuitem", { name: /重命名|Rename/ }),
    );
    const input = await screen.findByRole("textbox", {
      name: /重命名|Rename/,
    });
    fireEvent.change(input, { target: { value: "x".repeat(256) } });
    fireEvent.keyDown(input, { key: "Enter" });

    expect(state.sessions[0]?.title).toBeNull();
    expect(
      await screen.findByRole("textbox", { name: /重命名|Rename/ }),
    ).not.toBeNull();
  });

  it("keeps the rename editor open when persisting the title fails", async () => {
    const user = userEvent.setup();
    const state = workspaceWithOneSession();
    const client = createMockClient(state);
    client.session.rename = async () => {
      throw new LocalTransportError("tauri_invoke_failure", "offline");
    };
    renderSidebar(state, undefined, client);

    await waitFor(() => expect(treeRow(NEW_SESSION_LABEL)).not.toBeNull());
    await user.pointer({
      keys: "[MouseRight>]",
      target: treeRow(NEW_SESSION_LABEL)!,
    });
    await user.click(
      await screen.findByRole("menuitem", { name: /重命名|Rename/ }),
    );
    const input = await screen.findByRole("textbox", {
      name: /重命名|Rename/,
    });
    fireEvent.change(input, { target: { value: "Review auth" } });
    fireEvent.keyDown(input, { key: "Enter" });

    expect(state.sessions[0]?.title).toBeNull();
    expect(
      await screen.findByRole("textbox", { name: /重命名|Rename/ }),
    ).not.toBeNull();
  });

  it("renames a project from the context menu without opening a dialog", async () => {
    const user = userEvent.setup();
    const state = workspaceWithOneSession();
    renderSidebar(state);

    await waitFor(() => expect(treeRow(PROJECT.name)).not.toBeNull());
    await user.pointer({
      keys: "[MouseRight>]",
      target: treeRow(PROJECT.name)!,
    });
    await user.click(
      await screen.findByRole("menuitem", { name: /重命名|Rename/ }),
    );

    expect(useUiStore.getState().dialog).toBeNull();
    expect(screen.queryByRole("dialog")).toBeNull();
    const input = await screen.findByRole("textbox", {
      name: /重命名|Rename/,
    });
    fireEvent.change(input, { target: { value: "Ora Cloud" } });
    fireEvent.keyDown(input, { key: "Enter" });

    await waitFor(() => expect(state.projects[0]?.name).toBe("Ora Cloud"));
    await waitFor(() => expect(treeRow("Ora Cloud")).not.toBeNull());
  });

  it("renames a worktree from the context menu without status options", async () => {
    const user = userEvent.setup();
    const state = workspaceWithOneSession();
    const client = createMockClient(state);
    const update = vi.fn(client.task.update);
    client.task.update = update;
    renderSidebar(state, undefined, client);

    await waitFor(() => expect(treeRow(TASK.title)).not.toBeNull());
    await user.pointer({
      keys: "[MouseRight>]",
      target: treeRow(TASK.title)!,
    });
    expect(screen.queryByRole("menuitem", { name: /编辑|Edit/ })).toBeNull();
    await user.click(
      await screen.findByRole("menuitem", { name: /重命名|Rename/ }),
    );

    expect(useUiStore.getState().dialog).toBeNull();
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.queryByRole("combobox", { name: /状态|Status/ })).toBeNull();
    const input = await screen.findByRole("textbox", {
      name: /重命名|Rename/,
    });
    fireEvent.change(input, { target: { value: "Auth worktree" } });
    fireEvent.keyDown(input, { key: "Enter" });

    await waitFor(() => expect(state.tasks[0]?.title).toBe("Auth worktree"));
    expect(state.tasks[0]).toEqual({
      ...TASK,
      title: "Auth worktree",
    });
    expect(update.mock.calls[0]?.[0]).toEqual({
      taskId: TASK.id,
      title: "Auth worktree",
    });
    await waitFor(() => expect(treeRow("Auth worktree")).not.toBeNull());
  });

  it("renames a workflow run from the context menu without opening a dialog", async () => {
    const user = userEvent.setup();
    const state = workspaceWithOneSession();
    state.tasks = [
      TASK,
      {
        id: "wt1",
        projectId: PROJECT.id,
        title: "Review bot",
        workspaceMode: "worktree",
        type: "workflow",
        workflowRunId: "run1",
      },
    ];
    state.workflowRuns = [
      {
        id: "run1",
        projectId: PROJECT.id,
        workflowId: "wf1",
        snapshotId: "snap1",
        name: "Review bot",
        status: "pending",
        taskId: "wt1",
        createdAt: 0n,
        updatedAt: 0n,
      },
    ];
    renderSidebar(state);

    await waitFor(() => expect(treeRow("Review bot")).not.toBeNull());
    await user.pointer({
      keys: "[MouseRight>]",
      target: treeRow("Review bot")!,
    });
    await user.click(
      await screen.findByRole("menuitem", { name: /重命名|Rename/ }),
    );

    expect(useUiStore.getState().dialog).toBeNull();
    expect(screen.queryByRole("dialog")).toBeNull();
    const input = await screen.findByRole("textbox", {
      name: /重命名|Rename/,
    });
    fireEvent.change(input, { target: { value: "Review bot v2" } });
    fireEvent.keyDown(input, { key: "Enter" });

    await waitFor(() =>
      expect(state.workflowRuns[0]?.name).toBe("Review bot v2"),
    );
    await waitFor(() => expect(treeRow("Review bot v2")).not.toBeNull());
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

    await waitFor(() => expect(treeRow(NEW_SESSION_LABEL)).not.toBeNull());

    await user.click(screen.getByText(TASK.title));

    expect(treeRow(NEW_SESSION_LABEL)).toBeNull();
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

    await waitFor(() => expect(treeRow(NEW_SESSION_LABEL)).not.toBeNull());
    expect(workingIndicator()).toBeNull();
  });

  it("uses the same circle chat icon for direct chats and worktree sessions", async () => {
    const state = createMockClientState();
    state.projects = [PROJECT];
    state.tasks = [
      TASK,
      {
        id: "t2",
        projectId: PROJECT.id,
        title: "Direct chat",
        workspaceMode: "project_root",
        type: "default",
        workflowRunId: null,
      },
    ];
    state.sessions = [
      SESSION,
      {
        id: "s2",
        taskId: "t2",
        agentCli: "open_code",
        status: "running",
        title: "Direct chat",
        historyState: { type: "writable" },
      },
    ];
    renderSidebar(state);

    await waitFor(() => expect(treeRow("Direct chat")).not.toBeNull());
    expect(screen.getByLabelText(/直聊任务|Direct chat task/)).not.toBeNull();
    expect(
      screen.getByLabelText(/Git 工作树任务|Git worktree task/),
    ).not.toBeNull();
    expect(screen.getByLabelText(/^会话$|^Session$/)).not.toBeNull();
  });

  it("uses the persisted session title and ignores chat metadata for the row label", async () => {
    const state = workspaceWithOneSession();
    state.sessions = [{ ...SESSION, title: "Review auth flow" }];
    const store = createChatStore(
      createMockClient(createMockClientState()).session,
    );
    const { chatStore } = renderSidebar(state, store);

    act(() =>
      chatStore.setState({
        conversations: {
          [SESSION.id]: conversation({ sessionTitle: "Stale chat metadata" }),
        },
      }),
    );

    await waitFor(() => expect(treeRow("Review auth flow")).not.toBeNull());
    expect(treeRow("Stale chat metadata")).toBeNull();
  });

  it("searches persisted titles but not agent labels or the localized fallback", async () => {
    const user = userEvent.setup();
    const state = workspaceWithOneSession();
    state.sessions = [{ ...SESSION, title: "Review auth flow" }];
    renderSidebar(state);

    const search = screen.getByPlaceholderText(/搜索工作区|Search workspace/);
    await user.type(search, "Review auth flow");
    await waitFor(() => expect(treeRow("Review auth flow")).not.toBeNull());

    await user.clear(search);
    await user.type(search, "OpenCode");
    await waitFor(() =>
      expect(screen.getByText(/未找到项目|No projects found/)).not.toBeNull(),
    );

    await user.clear(search);
    await user.type(search, NEW_SESSION_LABEL.split("|")[0]!);
    await waitFor(() =>
      expect(screen.getByText(/未找到项目|No projects found/)).not.toBeNull(),
    );
  });

  it("shows the working indicator only while the session is responding", async () => {
    const store = createChatStore(
      createMockClient(createMockClientState()).session,
    );
    const { chatStore } = renderSidebar(workspaceWithOneSession(), store);
    await waitFor(() => expect(treeRow(NEW_SESSION_LABEL)).not.toBeNull());

    act(() =>
      chatStore.setState({
        conversations: { [SESSION.id]: conversation({ isResponding: true }) },
      }),
    );
    await waitFor(() => expect(workingIndicator()).not.toBeNull());

    act(() =>
      chatStore.setState({
        conversations: { [SESSION.id]: conversation({ isResponding: false }) },
      }),
    );
    await waitFor(() => expect(workingIndicator()).toBeNull());
  });

  // Matches the unread-mark aria-label in either shipped locale.
  const unreadMark = () => screen.queryByLabelText(/有未读更新|Unread/);

  it("shows an unread mark for an idle session flagged unread", async () => {
    useUnreadSessionsStore.setState({ unread: new Set([SESSION.id]) });
    renderSidebar(workspaceWithOneSession());

    await waitFor(() => expect(treeRow(NEW_SESSION_LABEL)).not.toBeNull());
    expect(unreadMark()).not.toBeNull();
    // The working animation is a distinct, higher-priority state.
    expect(workingIndicator()).toBeNull();
  });

  it("prefers the working animation over the unread mark while responding", async () => {
    useUnreadSessionsStore.setState({ unread: new Set([SESSION.id]) });
    const store = createChatStore(
      createMockClient(createMockClientState()).session,
    );
    const { chatStore } = renderSidebar(workspaceWithOneSession(), store);
    await waitFor(() => expect(treeRow(NEW_SESSION_LABEL)).not.toBeNull());

    act(() =>
      chatStore.setState({
        conversations: { [SESSION.id]: conversation({ isResponding: true }) },
      }),
    );

    await waitFor(() => expect(workingIndicator()).not.toBeNull());
    expect(unreadMark()).toBeNull();
  });
});
