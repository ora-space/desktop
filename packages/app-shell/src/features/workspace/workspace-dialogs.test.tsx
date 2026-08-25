import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { TooltipProvider } from "@ora/ui";
import { PlatformProvider } from "../../platform";
import { describe, expect, it, beforeEach } from "vitest";
import { AppI18nProvider } from "../../i18n/i18n";
import {
  createHookWrapper,
  createTestQueryClient,
} from "../../test/hook-harness";
import { createStubPlatform } from "../../test/stub-platform";
import { createChatStore } from "@ora/chat";
import {
  RemoteContractError,
  type ContractsClient,
  type Session,
} from "@ora/contracts";
import {
  createMockClient,
  createMockClientState,
} from "../../test/mock-client";
import { useUiStore } from "../../state/stores/ui-store";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import { useDraftSessionsStore } from "../../state/stores/draft-sessions-store";
import { WorkspaceDialogs } from "./workspace-dialogs";

beforeEach(() => {
  useUiStore.getState().setDialog(null);
  useUiStore.getState().setDeleteTarget(null);
  useWorkspaceSelectionStore.getState().clearSelection();
  useDraftSessionsStore.getState().clear();
});

describe("WorkspaceDialogs project creation", () => {
  it.each([
    ["C:\\workspace\\ora", "ora"],
    ["/workspace/ora/", "ora"],
  ])("derives the project name from %s", async (rootPath, expectedName) => {
    const user = userEvent.setup();
    const state = createMockClientState();
    const client = createMockClient(state);
    const chatStore = createChatStore(client.session);
    const Wrapper = createHookWrapper(
      client,
      createTestQueryClient(),
      chatStore,
    );
    useUiStore.getState().setDialog({ kind: "project" });

    render(
      <Wrapper>
        <AppI18nProvider>
          <PlatformProvider adapter={createStubPlatform()}>
            <TooltipProvider>
              <WorkspaceDialogs />
            </TooltipProvider>
          </PlatformProvider>
        </AppI18nProvider>
      </Wrapper>,
    );

    expect(screen.queryByLabelText(/项目名称|Project name/)).toBeNull();
    expect(
      screen.queryByText(
        /将代码仓库连接到 Ora 工作区|Connect a repository to the Ora workspace/,
      ),
    ).toBeNull();
    await user.type(
      screen.getByLabelText(/项目文件夹|Project folder/),
      rootPath,
    );
    await user.click(
      screen.getByRole("button", { name: /添加项目|Add project/ }),
    );

    await waitFor(() => {
      expect(state.projects).toEqual([
        {
          id: "p1",
          name: expectedName,
        },
      ]);
      expect(useUiStore.getState().dialog).toBeNull();
    });
    expect(useWorkspaceSelectionStore.getState().selection).toMatchObject({
      projectId: "p1",
      taskId: null,
      sessionId: null,
      workflowRunId: null,
    });
    expect(useWorkspaceSelectionStore.getState().selection.draftId).toEqual(
      expect.any(String),
    );
  });
});

describe("WorkspaceDialogs task creation", () => {
  it("creates only worktree tasks and does not offer a workspace-mode selector", async () => {
    const user = userEvent.setup();
    const state = createMockClientState();
    const baseClient = createMockClient(state);
    let submittedBaseBranch: string | undefined;
    let branchesLoaded = false;
    const client: ContractsClient = {
      ...baseClient,
      project: {
        ...baseClient.project,
        listBranches: async (request, options) => {
          const response = await baseClient.project.listBranches(
            request,
            options,
          );
          branchesLoaded = true;
          return response;
        },
      },
      task: {
        ...baseClient.task,
        create: async (request, options) => {
          submittedBaseBranch = request.baseBranch;
          return baseClient.task.create(request, options);
        },
      },
    };
    const Wrapper = createHookWrapper(
      client,
      createTestQueryClient(),
      createChatStore(client.session),
    );
    useUiStore.getState().setDialog({ kind: "task", projectId: "p1" });

    render(
      <Wrapper>
        <AppI18nProvider>
          <PlatformProvider adapter={createStubPlatform()}>
            <TooltipProvider>
              <WorkspaceDialogs />
            </TooltipProvider>
          </PlatformProvider>
        </AppI18nProvider>
      </Wrapper>,
    );

    expect(
      screen.queryByRole("combobox", { name: /工作区模式|Workspace mode/ }),
    ).toBeNull();
    expect(
      screen.getByText("Agent 在独立工作树中专注处理一项任务"),
    ).not.toBeNull();
    const titleInput = screen.getByLabelText(/任务标题|Task title/);
    expect(titleInput).not.toHaveAttribute("placeholder");
    expect(
      screen.getByRole("combobox", { name: /基础分支|Base branch/ }),
    ).not.toBeNull();
    await waitFor(() => expect(branchesLoaded).toBe(true));
    await user.type(titleInput, "Worktree task");
    await user.click(
      screen.getByRole("button", { name: /创建任务|Create task/ }),
    );

    await waitFor(() =>
      expect(state.tasks).toEqual([
        {
          id: "t1",
          projectId: "p1",
          workspaceId: "workspace-t1",
          title: "Worktree task",
        },
      ]),
    );
    expect(submittedBaseBranch).toBe("origin/main");
  });

  it("shows a spinner on the create button while worktree provisioning is in flight", async () => {
    const user = userEvent.setup();
    const state = createMockClientState();
    const baseClient = createMockClient(state);
    let releaseCreate: () => void = () => {};
    const createGate = new Promise<void>((resolve) => {
      releaseCreate = resolve;
    });
    const client: ContractsClient = {
      ...baseClient,
      task: {
        ...baseClient.task,
        create: async (request, options) => {
          await createGate;
          return baseClient.task.create(request, options);
        },
      },
    };
    const Wrapper = createHookWrapper(
      client,
      createTestQueryClient(),
      createChatStore(client.session),
    );
    useUiStore.getState().setDialog({ kind: "task", projectId: "p1" });

    render(
      <Wrapper>
        <AppI18nProvider>
          <PlatformProvider adapter={createStubPlatform()}>
            <TooltipProvider>
              <WorkspaceDialogs />
            </TooltipProvider>
          </PlatformProvider>
        </AppI18nProvider>
      </Wrapper>,
    );

    await waitFor(() => {
      expect(
        screen.getByRole("button", { name: /创建任务|Create task/ }),
      ).toBeEnabled();
    });
    await user.type(
      screen.getByLabelText(/任务标题|Task title/),
      "Slow worktree",
    );
    await user.click(
      screen.getByRole("button", { name: /创建任务|Create task/ }),
    );

    const submitButton = screen.getByRole("button", {
      name: /创建中|Creating/,
    });
    expect(submitButton).toBeDisabled();
    expect(submitButton).toHaveAttribute("aria-busy", "true");
    expect(submitButton.querySelector("[data-slot=spinner]")).not.toBeNull();
    expect(state.tasks).toEqual([]);

    releaseCreate();
    await waitFor(() =>
      expect(state.tasks).toEqual([
        {
          id: "t1",
          projectId: "p1",
          workspaceId: "workspace-t1",
          title: "Slow worktree",
        },
      ]),
    );
  });

  it("explains that worktree mode requires a Git repository", async () => {
    const user = userEvent.setup();
    const state = createMockClientState();
    const baseClient = createMockClient(state);
    const client: ContractsClient = {
      ...baseClient,
      task: {
        ...baseClient.task,
        create: async () => {
          throw new RemoteContractError(
            {
              code: "worktree_requires_git_repository",
              params: {},
              requestId: "550e8400-e29b-41d4-a716-446655440000",
            },
            null,
          );
        },
      },
    };
    const Wrapper = createHookWrapper(
      client,
      createTestQueryClient(),
      createChatStore(client.session),
    );
    useUiStore.getState().setDialog({ kind: "task", projectId: "p1" });

    render(
      <Wrapper>
        <AppI18nProvider>
          <PlatformProvider adapter={createStubPlatform()}>
            <TooltipProvider>
              <WorkspaceDialogs />
            </TooltipProvider>
          </PlatformProvider>
        </AppI18nProvider>
      </Wrapper>,
    );

    await waitFor(() => {
      expect(
        screen.getByRole("button", { name: /创建任务|Create task/ }),
      ).toBeEnabled();
    });
    await user.type(screen.getByLabelText(/任务标题|Task title/), "Needs Git");
    await user.click(
      screen.getByRole("button", { name: /创建任务|Create task/ }),
    );

    expect(await screen.findByRole("alert")).toHaveTextContent(
      /该目录不是 Git 仓库。请在 Git 仓库下创建 worktree 模式任务。|This directory is not a Git repository/,
    );
    expect(state.tasks).toEqual([]);
  });
});

describe("WorkspaceDialogs project deletion", () => {
  it("deletes every descendant session before deleting the project", async () => {
    const user = userEvent.setup();
    const state = createMockClientState();
    state.projects = [{ id: "p1", name: "Ora" }];
    state.sessions = [
      {
        id: "s1",
        workspaceId: "workspace-t1",
        agentRef: "ora-space.opencode",
        status: "running",
        title: null,
        historyState: { type: "writable" },
      },
      {
        id: "s2",
        workspaceId: "workspace-t2",
        agentRef: "ora-space.opencode",
        status: "running",
        title: null,
        historyState: { type: "writable" },
      },
    ];
    const calls: string[] = [];
    const baseClient = createMockClient(state);
    const client: ContractsClient = {
      ...baseClient,
      project: {
        ...baseClient.project,
        delete: async (request, options) => {
          calls.push(`project:${request.projectId}`);
          return baseClient.project.delete(request, options);
        },
      },
      session: {
        ...baseClient.session,
        delete: async (request, options) => {
          calls.push(`session:${request.sessionId}`);
          return baseClient.session.delete(request, options);
        },
      },
    };
    const Wrapper = createHookWrapper(
      client,
      createTestQueryClient(),
      createChatStore(client.session),
    );
    useUiStore.getState().setDeleteTarget({
      kind: "project",
      id: "p1",
      name: "Ora",
      sessionIds: ["s1", "s2"],
    });

    render(
      <Wrapper>
        <AppI18nProvider>
          <PlatformProvider adapter={createStubPlatform()}>
            <TooltipProvider>
              <WorkspaceDialogs />
            </TooltipProvider>
          </PlatformProvider>
        </AppI18nProvider>
      </Wrapper>,
    );

    await user.click(screen.getByRole("button", { name: /^删除$|^Delete$/ }));

    await waitFor(() => {
      expect(calls).toEqual(["session:s1", "session:s2", "project:p1"]);
      expect(state.sessions).toEqual([]);
      expect(state.projects).toEqual([]);
    });
  });
});

describe("WorkspaceDialogs task deletion", () => {
  it("deletes every task session before deleting its worktree task", async () => {
    const sessionIds = ["s1", "s2"] as const;
    const description =
      "该任务的会话记录、Git 工作树及其 ora/* 分支将被删除。未提交的修改和仅存在于该分支的提交将永久丢失，此操作无法撤销。";
    const user = userEvent.setup();
    const state = createMockClientState();
    state.tasks = [
      {
        id: "t1",
        projectId: "p1",
        workspaceId: "workspace-t1",
        title: "Delete me",
      },
    ];
    state.sessions = sessionIds.map((id): Session => ({
      id,
      workspaceId: "workspace-t1",
      agentRef: "ora-space.opencode",
      status: "running",
      title: null,
      historyState: { type: "writable" },
    }));
    const calls: string[] = [];
    const baseClient = createMockClient(state);
    const client: ContractsClient = {
      ...baseClient,
      task: {
        ...baseClient.task,
        delete: async (request, options) => {
          calls.push(`task:${request.taskId}`);
          return baseClient.task.delete(request, options);
        },
      },
      session: {
        ...baseClient.session,
        delete: async (request, options) => {
          calls.push(`session:${request.sessionId}`);
          return baseClient.session.delete(request, options);
        },
      },
    };
    const Wrapper = createHookWrapper(
      client,
      createTestQueryClient(),
      createChatStore(client.session),
    );
    useUiStore.getState().setDeleteTarget({
      kind: "task",
      id: "t1",
      name: "Delete me",
      sessionIds: [...sessionIds],
    });

    render(
      <Wrapper>
        <AppI18nProvider>
          <PlatformProvider adapter={createStubPlatform()}>
            <TooltipProvider>
              <WorkspaceDialogs />
            </TooltipProvider>
          </PlatformProvider>
        </AppI18nProvider>
      </Wrapper>,
    );

    expect(screen.getByText(description)).not.toBeNull();
    await user.click(screen.getByRole("button", { name: /^删除$|^Delete$/ }));

    await waitFor(() => {
      expect(calls).toEqual([
        ...sessionIds.map((id) => `session:${id}`),
        "task:t1",
      ]);
      expect(state.sessions).toEqual([]);
      expect(state.tasks).toEqual([]);
    });
  });

  it("uses the standard resource-in-use error for worktree tasks", async () => {
    const expectedError = /无法删除，请先停止正在运行的会话|Unable to delete/;
    const user = userEvent.setup();
    const state = createMockClientState();
    const baseClient = createMockClient(state);
    const client: ContractsClient = {
      ...baseClient,
      task: {
        ...baseClient.task,
        delete: async () => {
          throw new RemoteContractError(
            {
              code: "resource_in_use",
              params: {},
              requestId: "550e8400-e29b-41d4-a716-446655440000",
            },
            null,
          );
        },
      },
    };
    const Wrapper = createHookWrapper(
      client,
      createTestQueryClient(),
      createChatStore(client.session),
    );
    useUiStore.getState().setDeleteTarget({
      kind: "task",
      id: "t1",
      name: "Delete me",
      sessionIds: [],
    });

    render(
      <Wrapper>
        <AppI18nProvider>
          <PlatformProvider adapter={createStubPlatform()}>
            <TooltipProvider>
              <WorkspaceDialogs />
            </TooltipProvider>
          </PlatformProvider>
        </AppI18nProvider>
      </Wrapper>,
    );

    await user.click(screen.getByRole("button", { name: /^删除$|^Delete$/ }));

    expect(await screen.findByRole("alert")).toHaveTextContent(expectedError);
  });
});
