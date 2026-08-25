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
  type ContractsClient,
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
import { useDraftSessionsStore } from "../../state/stores/draft-sessions-store";
import { useUnreadSessionsStore } from "../../state/stores/unread-sessions-store";
import { dismissSessionDraft } from "../../state/session-drafts";
import { WorkspaceSidebar } from "./workspace-sidebar";

const USER = { name: "Eric", email: "eric@example.com" };
// Deliberately not "Ora": the sidebar header renders that as the product mark,
// so a project of the same name makes every text query ambiguous.
const PROJECT: Project = { id: "p1", name: "Ora Desktop" };
const TASK: Task = {
  id: "t1",
  projectId: "p1",
  workspaceId: "workspace-t1",
  title: "Refactor",
};
const SESSION: Session = {
  id: "s1",
  workspaceId: "workspace-t1",
  agentRef: "ora-space.opencode",
  status: "running",
  title: null,
  historyState: { type: "writable" },
};
const DIRECT_SESSION: Session = {
  id: "s-direct",
  workspaceId: "workspace-p1",
  agentRef: "ora-space.opencode",
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
  window.localStorage.clear();
  useWorkspaceSelectionStore.setState({
    selection: {
      projectId: null,
      taskId: null,
      sessionId: null,
      workflowRunId: null,
      draftId: null,
    },
    pendingRestore: null,
    createFocus: null,
  });
  useDraftSessionsStore.getState().clear();
  useUiStore.setState({
    sidebarCollapsed: false,
    expandedProjects: new Set(),
    expandedTasks: new Set(),
    treeExpansionBootstrapped: false,
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

/** Outer TreeRow shell that also hosts the hover plus / archive controls. */
function treeRowShell(label: string): HTMLElement {
  const row = treeRow(label);
  expect(row).not.toBeNull();
  const shell = row!.closest(".group\\/tree");
  expect(shell).not.toBeNull();
  return shell as HTMLElement;
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
      draftId: null,
    });
    expect(useWorkspaceSelectionStore.getState().createFocus).toEqual({
      projectId: PROJECT.id,
      taskId: null,
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

    // Selection was a worktree session, so New chat lands under that worktree.
    expect(useWorkspaceSelectionStore.getState().selection).toMatchObject({
      projectId: PROJECT.id,
      taskId: TASK.id,
      sessionId: null,
      workflowRunId: null,
    });
    expect(useWorkspaceSelectionStore.getState().selection.draftId).toEqual(
      expect.any(String),
    );
    await waitFor(() => expect(treeRow(NEW_SESSION_LABEL)).not.toBeNull());
  });

  it("creates under the project last clicked even while another session stays selected", async () => {
    const user = userEvent.setup();
    const other: Project = {
      id: "p2",
      name: "Other App",
    };
    const state = workspaceWithOneSession();
    state.projects = [PROJECT, other];
    useWorkspaceSelectionStore
      .getState()
      .selectSession(SESSION.id, TASK.id, PROJECT.id);
    renderSidebar(state);

    await waitFor(() => expect(treeRow(other.name)).not.toBeNull());
    await user.click(screen.getByText(other.name));
    expect(useWorkspaceSelectionStore.getState().selection.sessionId).toBe(
      SESSION.id,
    );
    expect(useWorkspaceSelectionStore.getState().createFocus).toEqual({
      projectId: other.id,
      taskId: null,
    });

    await user.click(
      await screen.findByRole("button", { name: /新建对话|New chat/ }),
    );

    expect(useWorkspaceSelectionStore.getState().selection).toMatchObject({
      projectId: other.id,
      taskId: null,
      sessionId: null,
      workflowRunId: null,
    });
    expect(useWorkspaceSelectionStore.getState().selection.draftId).toEqual(
      expect.any(String),
    );
  });

  it("creates under a worktree after clicking that worktree row", async () => {
    const user = userEvent.setup();
    const state = workspaceWithOneSession();
    // Preselect a live session so a regression that wrongly routes the worktree
    // row through selectTask (yanking the composer) is caught: starting from
    // an empty selection, a null-sessionId assertion would pass either way.
    useWorkspaceSelectionStore
      .getState()
      .selectSession(SESSION.id, TASK.id, PROJECT.id);
    renderSidebar(state);

    await waitFor(() => expect(treeRow(TASK.title)).not.toBeNull());
    await user.click(screen.getByText(TASK.title));
    // Clicking a worktree row only retargets New chat; it must not move the
    // composer off the live session.
    expect(useWorkspaceSelectionStore.getState().selection.sessionId).toBe(
      SESSION.id,
    );
    expect(useWorkspaceSelectionStore.getState().createFocus).toEqual({
      projectId: PROJECT.id,
      taskId: TASK.id,
    });

    await user.click(
      await screen.findByRole("button", { name: /新建对话|New chat/ }),
    );

    expect(useWorkspaceSelectionStore.getState().selection).toMatchObject({
      projectId: PROJECT.id,
      taskId: TASK.id,
      sessionId: null,
      workflowRunId: null,
    });
    expect(useWorkspaceSelectionStore.getState().selection.draftId).toEqual(
      expect.any(String),
    );
  });

  it("keeps New chat visible when the selected chat belongs to the main workspace", async () => {
    const user = userEvent.setup();
    const state = createMockClientState();
    state.projects = [PROJECT];
    state.tasks = [];
    state.sessions = [DIRECT_SESSION];
    // Ordinary sessions select the project directly because they have no Task.
    useWorkspaceSelectionStore
      .getState()
      .selectSessionBeforeTask(DIRECT_SESSION.id, PROJECT.id);
    renderSidebar(state);

    await user.click(
      await screen.findByRole("button", { name: /新建对话|New chat/ }),
    );

    expect(useWorkspaceSelectionStore.getState().selection).toMatchObject({
      projectId: PROJECT.id,
      taskId: null,
      sessionId: null,
      workflowRunId: null,
    });
    expect(useWorkspaceSelectionStore.getState().selection.draftId).toEqual(
      expect.any(String),
    );
    await waitFor(() => expect(treeRow(NEW_SESSION_LABEL)).not.toBeNull());
  });

  it("discards an empty draft when another session is selected", async () => {
    const user = userEvent.setup();
    const state = workspaceWithOneSession();
    state.sessions = [{ ...SESSION, title: "Review auth" }];
    useWorkspaceSelectionStore
      .getState()
      .selectSession(SESSION.id, TASK.id, PROJECT.id);
    renderSidebar(state);

    await user.click(
      await screen.findByRole("button", { name: /新建对话|New chat/ }),
    );
    await waitFor(() => expect(treeRow(NEW_SESSION_LABEL)).not.toBeNull());

    await user.click(screen.getByText("Review auth"));

    expect(useWorkspaceSelectionStore.getState().selection).toEqual({
      projectId: PROJECT.id,
      taskId: TASK.id,
      sessionId: SESSION.id,
      workflowRunId: null,
      draftId: null,
    });
    expect(treeRow(NEW_SESSION_LABEL)).toBeNull();
    expect(useDraftSessionsStore.getState().drafts).toHaveLength(0);
  });

  it("keeps a typed draft until it is dismissed", async () => {
    const user = userEvent.setup();
    const state = workspaceWithOneSession();
    state.sessions = [{ ...SESSION, title: "Review auth" }];
    renderSidebar(state);

    await user.click(
      await screen.findByRole("button", { name: /新建对话|New chat/ }),
    );
    const draftId = useWorkspaceSelectionStore.getState().selection.draftId;
    expect(draftId).toEqual(expect.any(String));
    act(() => {
      useDraftSessionsStore
        .getState()
        .updateContent(draftId!, { text: "keep this" });
    });

    await waitFor(() => expect(treeRow("keep this")).not.toBeNull());
    await user.click(screen.getByText("Review auth"));
    expect(treeRow("keep this")).not.toBeNull();
    expect(useDraftSessionsStore.getState().drafts).toHaveLength(1);

    await user.click(
      screen.getByRole("button", { name: /关闭草稿|Dismiss draft/ }),
    );
    expect(treeRow("keep this")).toBeNull();
    expect(useDraftSessionsStore.getState().drafts).toHaveLength(0);
    await waitFor(() =>
      expect(document.activeElement?.getAttribute("role")).toBe("button"),
    );
  });

  it("starts a muted draft from the worktree plus, not the row click", async () => {
    const user = userEvent.setup();
    useWorkspaceSelectionStore
      .getState()
      .selectSession(SESSION.id, TASK.id, PROJECT.id);
    renderSidebar(workspaceWithOneSession());

    await waitFor(() => expect(treeRow(TASK.title)).not.toBeNull());
    await user.click(
      within(treeRowShell(TASK.title)).getByRole("button", {
        name: /^新建会话$|^New session$/,
      }),
    );

    expect(useWorkspaceSelectionStore.getState().selection).toMatchObject({
      projectId: PROJECT.id,
      taskId: TASK.id,
      sessionId: null,
      workflowRunId: null,
    });
    expect(useWorkspaceSelectionStore.getState().selection.draftId).toEqual(
      expect.any(String),
    );
    await waitFor(() => expect(treeRow(NEW_SESSION_LABEL)).not.toBeNull());
  });

  it("collapses a worktree on click without discarding an existing draft", async () => {
    const user = userEvent.setup();
    renderSidebar(workspaceWithOneSession());

    await waitFor(() => expect(treeRow(NEW_SESSION_LABEL)).not.toBeNull());
    await user.click(
      within(treeRowShell(TASK.title)).getByRole("button", {
        name: /^新建会话$|^New session$/,
      }),
    );
    expect(treeRow(NEW_SESSION_LABEL)).not.toBeNull();

    await user.click(screen.getByText(TASK.title));

    expect(useUiStore.getState().expandedTasks.has(TASK.id)).toBe(false);
    expect(treeRow(NEW_SESSION_LABEL)).toBeNull();
    expect(useWorkspaceSelectionStore.getState().selection.draftId).toEqual(
      expect.any(String),
    );
  });

  it("opens the live session when a bound draft row is clicked", async () => {
    const user = userEvent.setup();
    const state = workspaceWithOneSession();
    state.sessions = [{ ...SESSION, title: "Review auth" }];
    renderSidebar(state);

    await waitFor(() => expect(treeRow(TASK.title)).not.toBeNull());
    await user.click(
      within(treeRowShell(TASK.title)).getByRole("button", {
        name: /^新建会话$|^New session$/,
      }),
    );
    const draftId = useWorkspaceSelectionStore.getState().selection.draftId!;
    act(() => {
      useDraftSessionsStore.getState().updateContent(draftId, {
        text: "in flight",
      });
      useDraftSessionsStore.getState().bindToSession(draftId, "pending-s");
      useWorkspaceSelectionStore
        .getState()
        .selectSession("pending-s", TASK.id, PROJECT.id);
    });

    await user.click(screen.getByText("Review auth"));
    await waitFor(() => expect(treeRow("in flight")).not.toBeNull());
    await user.click(screen.getByText("in flight"));

    expect(useWorkspaceSelectionStore.getState().selection).toEqual({
      projectId: PROJECT.id,
      taskId: TASK.id,
      sessionId: "pending-s",
      workflowRunId: null,
      draftId: null,
    });
  });

  it("moves selection onto a bound draft's persisted session before removing the row", async () => {
    const draftId = useDraftSessionsStore
      .getState()
      .ensureEmptyDraft({ projectId: PROJECT.id, taskId: TASK.id });
    useDraftSessionsStore
      .getState()
      .updateContent(draftId, { text: "sending" });
    useDraftSessionsStore.getState().bindToSession(draftId, SESSION.id);
    useWorkspaceSelectionStore
      .getState()
      .selectDraft(draftId, TASK.id, PROJECT.id);

    renderSidebar(workspaceWithOneSession());

    await waitFor(() =>
      expect(useWorkspaceSelectionStore.getState().selection).toEqual({
        projectId: PROJECT.id,
        taskId: TASK.id,
        sessionId: SESSION.id,
        workflowRunId: null,
        draftId: null,
      }),
    );
    expect(useDraftSessionsStore.getState().drafts).toEqual([]);
  });

  it("keeps the live session selected when a bound draft is dismissed", async () => {
    const id = useDraftSessionsStore
      .getState()
      .ensureEmptyDraft({ projectId: PROJECT.id, taskId: TASK.id });
    useDraftSessionsStore.getState().updateContent(id, { text: "sending" });
    // Bind to a warm id that is not persisted yet so the muted row stays visible.
    useDraftSessionsStore.getState().bindToSession(id, "pending-s");
    useWorkspaceSelectionStore
      .getState()
      .selectSession("pending-s", TASK.id, PROJECT.id);
    useUiStore.getState().expandProject(PROJECT.id);
    useUiStore.getState().expandTask(TASK.id);
    renderSidebar(workspaceWithOneSession());

    await waitFor(() => expect(treeRow("sending")).not.toBeNull());
    expect(
      screen.queryByRole("button", { name: /关闭草稿|Dismiss draft/ }),
    ).toBeNull();

    act(() => {
      dismissSessionDraft(id);
    });

    expect(useDraftSessionsStore.getState().drafts).toHaveLength(0);
    expect(useWorkspaceSelectionStore.getState().selection).toEqual({
      projectId: PROJECT.id,
      taskId: TASK.id,
      sessionId: "pending-s",
      workflowRunId: null,
      draftId: null,
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

    expect(useWorkspaceSelectionStore.getState().selection).toMatchObject({
      projectId: PROJECT.id,
      taskId: null,
      sessionId: null,
      workflowRunId: null,
    });
    expect(useWorkspaceSelectionStore.getState().selection.draftId).toEqual(
      expect.any(String),
    );
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
    state.sessions.push({
      id: "s2",
      workspaceId: "workspace-p1",
      agentRef: "ora-space.opencode",
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
      sessionIds: ["s2", "s1"],
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

    expect(useWorkspaceSelectionStore.getState().selection).toMatchObject({
      projectId: PROJECT.id,
      taskId: null,
      sessionId: null,
      workflowRunId: null,
    });
    expect(useWorkspaceSelectionStore.getState().selection.draftId).toEqual(
      expect.any(String),
    );
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
        workspaceId: "workspace-wt1",
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
        name: /运行工作流|Run workflow/,
      }),
    );
    await user.click(
      await screen.findByRole("button", {
        name: /运行工作流|Run workflow/,
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
      name: /运行工作流|Run workflow/,
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
    const baseClient = createMockClient(state);
    const renameCalls: string[] = [];
    const client: ContractsClient = {
      ...baseClient,
      workflowRun: {
        ...baseClient.workflowRun,
        rename: async (request, options) => {
          renameCalls.push(request.name);
          return baseClient.workflowRun.rename(request, options);
        },
      },
    };
    state.tasks = [
      TASK,
      {
        id: "wt1",
        projectId: PROJECT.id,
        workspaceId: "workspace-wt1",
        title: "Workflow host",
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
        workspaceId: "workspace-wt1",
        createdAt: 0n,
        updatedAt: 0n,
      },
    ];
    renderSidebar(state, undefined, client);

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
    await user.clear(input);
    await user.type(input, "Review bot v2");
    await user.keyboard("{Enter}");

    await waitFor(() => expect(renameCalls).toEqual(["Review bot v2"]));
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

  it("collapses a worktree on the first click, not the second", async () => {
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

  // First collapse unmounts; after a reopen, collapse hides instead of remounting.
  it("unmounts on the first collapse and retains after a reopen", async () => {
    const user = userEvent.setup();
    renderSidebar(workspaceWithOneSession());

    await waitFor(() => expect(treeRow(TASK.title)).not.toBeNull());

    await user.click(screen.getByText(PROJECT.name));
    expect(treeRow(TASK.title)).toBeNull();
    expect(screen.queryByText(TASK.title)).toBeNull();

    await user.click(screen.getByText(PROJECT.name));
    await waitFor(() => expect(treeRow(TASK.title)).not.toBeNull());

    await user.click(screen.getByText(PROJECT.name));
    expect(treeRow(TASK.title)).toBeNull();
    expect(screen.getByText(TASK.title).closest("[hidden]")).not.toBeNull();
  });

  it("keeps a previously collapsed project collapsed after hydrate", async () => {
    useUiStore.setState({
      expandedProjects: new Set(),
      expandedTasks: new Set(),
      treeExpansionBootstrapped: true,
    });
    renderSidebar(workspaceWithOneSession());

    await waitFor(() => expect(treeRow(PROJECT.name)).not.toBeNull());
    expect(treeRow(TASK.title)).toBeNull();
    expect(useUiStore.getState().expandedProjects.has(PROJECT.id)).toBe(false);
  });

  it("does not re-expand a collapsed project when restoring its selected session", async () => {
    useUiStore.setState({
      expandedProjects: new Set(),
      expandedTasks: new Set(),
      treeExpansionBootstrapped: true,
    });
    useWorkspaceSelectionStore.setState({
      selection: {
        projectId: null,
        taskId: null,
        sessionId: null,
        workflowRunId: null,
        draftId: null,
      },
      pendingRestore: {
        projectId: PROJECT.id,
        taskId: TASK.id,
        sessionId: SESSION.id,
        workflowRunId: null,
        draftId: null,
      },
      createFocus: null,
    });
    renderSidebar(workspaceWithOneSession());

    await waitFor(() =>
      expect(useWorkspaceSelectionStore.getState().selection.sessionId).toBe(
        SESSION.id,
      ),
    );
    expect(treeRow(TASK.title)).toBeNull();
    expect(useUiStore.getState().expandedProjects.has(PROJECT.id)).toBe(false);
    expect(treeRowShell(PROJECT.name).dataset.selectionHint).toBe("true");
  });

  it("bubbles selection hint to collapsed ancestors and clears it on expand", async () => {
    const user = userEvent.setup();
    useUiStore.setState({
      expandedProjects: new Set([PROJECT.id]),
      expandedTasks: new Set([TASK.id]),
      treeExpansionBootstrapped: true,
    });
    useWorkspaceSelectionStore
      .getState()
      .selectSession(SESSION.id, TASK.id, PROJECT.id);
    renderSidebar(workspaceWithOneSession());

    await waitFor(() => expect(treeRow(NEW_SESSION_LABEL)).not.toBeNull());
    expect(treeRowShell(PROJECT.name).dataset.selectionHint).toBeUndefined();
    expect(treeRowShell(TASK.title).dataset.selectionHint).toBeUndefined();

    await user.click(screen.getByText(TASK.title));
    expect(useUiStore.getState().expandedTasks.has(TASK.id)).toBe(false);
    expect(treeRow(NEW_SESSION_LABEL)).toBeNull();
    expect(treeRowShell(TASK.title).dataset.selectionHint).toBe("true");
    expect(treeRowShell(PROJECT.name).dataset.selectionHint).toBeUndefined();
    expect(useWorkspaceSelectionStore.getState().selection.sessionId).toBe(
      SESSION.id,
    );

    await user.click(screen.getByText(PROJECT.name));
    expect(useUiStore.getState().expandedProjects.has(PROJECT.id)).toBe(false);
    expect(treeRow(TASK.title)).toBeNull();
    expect(treeRowShell(PROJECT.name).dataset.selectionHint).toBe("true");
    expect(useWorkspaceSelectionStore.getState().selection.sessionId).toBe(
      SESSION.id,
    );

    await user.click(screen.getByText(PROJECT.name));
    await waitFor(() => expect(treeRow(TASK.title)).not.toBeNull());
    expect(treeRowShell(PROJECT.name).dataset.selectionHint).toBeUndefined();
    expect(treeRowShell(TASK.title).dataset.selectionHint).toBe("true");

    await user.click(screen.getByText(TASK.title));
    await waitFor(() => expect(treeRow(NEW_SESSION_LABEL)).not.toBeNull());
    expect(treeRowShell(TASK.title).dataset.selectionHint).toBeUndefined();
  });

  it("does not show a selection hint when the project itself is selected", async () => {
    useUiStore.setState({
      expandedProjects: new Set(),
      expandedTasks: new Set(),
      treeExpansionBootstrapped: true,
    });
    useWorkspaceSelectionStore.getState().selectProject(PROJECT.id);
    renderSidebar(workspaceWithOneSession());

    await waitFor(() => expect(treeRow(PROJECT.name)).not.toBeNull());
    expect(treeRowShell(PROJECT.name).dataset.selectionHint).toBeUndefined();
    expect(
      treeRowShell(PROJECT.name).className.includes("bg-sidebar-accent "),
    ).toBe(true);
  });

  it("does not seal first-run bootstrap when the tree query fails", async () => {
    const state = workspaceWithOneSession();
    const client = createMockClient(state);
    vi.spyOn(client.project, "list").mockRejectedValue(
      new LocalTransportError("tauri_invoke_failure", "projects unavailable"),
    );
    renderSidebar(state, undefined, client);

    await waitFor(() =>
      expect(
        screen.getByText(/projects unavailable|调用失败|tauri/i),
      ).toBeTruthy(),
    );
    expect(useUiStore.getState().treeExpansionBootstrapped).toBe(false);
    expect(useUiStore.getState().expandedProjects.size).toBe(0);
  });

  it("keeps a staged session restore when the sessions query fails", async () => {
    useWorkspaceSelectionStore.setState({
      selection: {
        projectId: null,
        taskId: null,
        sessionId: null,
        workflowRunId: null,
        draftId: null,
      },
      pendingRestore: {
        projectId: PROJECT.id,
        taskId: TASK.id,
        sessionId: SESSION.id,
        workflowRunId: null,
        draftId: null,
      },
      createFocus: null,
    });
    const state = workspaceWithOneSession();
    const client = createMockClient(state);
    vi.spyOn(client.session, "list").mockRejectedValue(
      new LocalTransportError("tauri_invoke_failure", "sessions unavailable"),
    );
    renderSidebar(state, undefined, client);

    await waitFor(() =>
      expect(
        screen.getByText(/sessions unavailable|调用失败|tauri/i),
      ).toBeTruthy(),
    );
    expect(
      useWorkspaceSelectionStore.getState().pendingRestore?.sessionId,
    ).toBe(SESSION.id);
    expect(
      useWorkspaceSelectionStore.getState().selection.sessionId,
    ).toBeNull();
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
    state.tasks = [TASK];
    state.sessions = [
      SESSION,
      {
        id: "s2",
        workspaceId: "workspace-p1",
        agentRef: "ora-space.opencode",
        status: "running",
        title: "Direct chat",
        historyState: { type: "writable" },
      },
    ];
    renderSidebar(state);

    await waitFor(() => expect(treeRow("Direct chat")).not.toBeNull());
    expect(
      screen.getByLabelText(/Git 工作树任务|Git worktree task/),
    ).not.toBeNull();
    expect(screen.getAllByLabelText(/^会话$|^Session$/)).toHaveLength(2);
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

  it("reacts to structured draft title changes while search is active", async () => {
    const user = userEvent.setup();
    const draftId = useDraftSessionsStore
      .getState()
      .ensureEmptyDraft({ projectId: PROJECT.id, taskId: null });
    useDraftSessionsStore
      .getState()
      .updateContent(draftId, { text: "Needle draft" });
    renderSidebar(workspaceWithOneSession());

    const search = screen.getByPlaceholderText(/搜索工作区|Search workspace/);
    await user.type(search, "Needle draft");
    expect(treeRow(PROJECT.name)).not.toBeNull();

    act(() => {
      useDraftSessionsStore
        .getState()
        .updateContent(draftId, { text: "Different title" });
    });
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

  it("hides the working indicator while session history is still loading", async () => {
    const store = createChatStore(
      createMockClient(createMockClientState()).session,
    );
    const { chatStore } = renderSidebar(workspaceWithOneSession(), store);
    await waitFor(() => expect(treeRow(NEW_SESSION_LABEL)).not.toBeNull());

    act(() =>
      chatStore.setState({
        conversations: {
          [SESSION.id]: conversation({
            isLoading: true,
            isResponding: true,
            isLoaded: false,
          }),
        },
      }),
    );
    await waitFor(() => expect(workingIndicator()).toBeNull());

    act(() =>
      chatStore.setState({
        conversations: {
          [SESSION.id]: conversation({
            isLoading: false,
            isResponding: true,
            isLoaded: true,
          }),
        },
      }),
    );
    await waitFor(() => expect(workingIndicator()).not.toBeNull());
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
