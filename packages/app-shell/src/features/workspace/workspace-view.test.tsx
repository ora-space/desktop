import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createChatStore } from "@ora/chat";
import type { ContractsClient } from "@ora/contracts";
import { TooltipProvider } from "@ora/ui";
import { PlatformProvider } from "@ora/platform";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AppI18nProvider } from "../../i18n/i18n";
import {
  createHookWrapper,
  createTestQueryClient,
} from "../../test/hook-harness";
import { createStubPlatform } from "../../test/stub-platform";
import {
  createMockClient,
  createMockClientState,
} from "../../test/mock-client";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import { directChatTitle, WorkspaceView } from "./workspace-view";

beforeEach(() => {
  useWorkspaceSelectionStore.getState().clearSelection();
});

describe("WorkspaceView", () => {
  it("reloads a selected running session after the in-memory chat store is recreated", async () => {
    const state = createMockClientState();
    state.projects = [{ id: "p1", name: "Ora", rootPath: "/ora" }];
    state.tasks = [
      {
        id: "t1",
        projectId: "p1",
        title: "Refresh history",
        status: "todo",
        workspaceMode: "worktree",
      },
    ];
    state.sessions = [
      {
        id: "s1",
        taskId: "t1",
        agentCli: "open_code",
        status: "running",
        historyState: { type: "writable" },
      },
    ];
    const client = createMockClient(state);
    const load = vi.fn(async function* () {
      yield { type: "completed" as const };
    });
    client.session.load = load;
    const chatStore = createChatStore(client.session);
    const Wrapper = createHookWrapper(
      client,
      createTestQueryClient(),
      chatStore,
    );
    useWorkspaceSelectionStore.getState().selectSession("s1", "t1", "p1");

    render(
      <Wrapper>
        <AppI18nProvider>
          <PlatformProvider adapter={createStubPlatform()}>
            <TooltipProvider>
              <WorkspaceView userName="Eric" />
            </TooltipProvider>
          </PlatformProvider>
        </AppI18nProvider>
      </Wrapper>,
    );

    await waitFor(() => expect(load).toHaveBeenCalledOnce());
    await waitFor(() =>
      expect(chatStore.getState().conversations.s1?.isLoaded).toBe(true),
    );
  });

  it("does not load history for a newly initialized session", async () => {
    const state = createMockClientState();
    state.projects = [{ id: "p1", name: "Ora", rootPath: "/ora" }];
    state.tasks = [
      {
        id: "t1",
        projectId: "p1",
        title: "Direct chat",
        status: "todo",
        workspaceMode: "project_root",
      },
    ];
    state.sessions = [
      {
        id: "s1",
        taskId: "t1",
        agentCli: "open_code",
        status: "running",
        historyState: { type: "writable" },
      },
    ];
    const client = createMockClient(state);
    const load = vi.fn(async function* () {
      yield { type: "completed" as const };
    });
    client.session.load = load;
    const chatStore = createChatStore(client.session);
    chatStore.getState().initializeSession("s1");
    const Wrapper = createHookWrapper(
      client,
      createTestQueryClient(),
      chatStore,
    );
    useWorkspaceSelectionStore.getState().selectSession("s1", "t1", "p1");

    render(
      <Wrapper>
        <AppI18nProvider>
          <PlatformProvider adapter={createStubPlatform()}>
            <TooltipProvider>
              <WorkspaceView userName="Eric" />
            </TooltipProvider>
          </PlatformProvider>
        </AppI18nProvider>
      </Wrapper>,
    );

    await waitFor(() =>
      expect(chatStore.getState().conversations.s1?.isLoaded).toBe(true),
    );
    expect(load).not.toHaveBeenCalled();
  });

  it("keeps the composer disabled when no project is selected", async () => {
    const state = createMockClientState();
    const client = createMockClient(state);
    const Wrapper = createHookWrapper(
      client,
      createTestQueryClient(),
      createChatStore(client.session),
    );

    render(
      <Wrapper>
        <AppI18nProvider>
          <PlatformProvider adapter={createStubPlatform()}>
            <TooltipProvider>
              <WorkspaceView userName="Eric" />
            </TooltipProvider>
          </PlatformProvider>
        </AppI18nProvider>
      </Wrapper>,
    );

    expect(await screen.findByRole("textbox")).toBeDisabled();
  });

  it("does not repeat the default direct-chat mode in the composer context", async () => {
    const state = createMockClientState();
    state.projects = [{ id: "p1", name: "Ora", rootPath: "/ora" }];
    const client = createMockClient(state);
    const Wrapper = createHookWrapper(
      client,
      createTestQueryClient(),
      createChatStore(client.session),
    );
    useWorkspaceSelectionStore.getState().selectProject("p1");

    render(
      <Wrapper>
        <AppI18nProvider>
          <PlatformProvider adapter={createStubPlatform()}>
            <TooltipProvider>
              <WorkspaceView userName="Eric" />
            </TooltipProvider>
          </PlatformProvider>
        </AppI18nProvider>
      </Wrapper>,
    );

    await waitFor(() => expect(screen.getByRole("textbox")).toBeEnabled());
    expect(
      screen.getByRole("button", { name: /选择项目|Select project/ }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /选择启动模式|Select launch mode/ }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /选择分支|Select branch/ }),
    ).toBeNull();
    expect(screen.queryByText(/直聊|Direct chat/)).toBeNull();
  });

  it("shows only worktrees in the worktree context menu", async () => {
    const user = userEvent.setup();
    const state = createMockClientState();
    state.projects = [{ id: "p1", name: "Ora", rootPath: "/ora" }];
    state.tasks = [
      {
        id: "t1",
        projectId: "p1",
        title: "Current worktree",
        status: "todo",
        workspaceMode: "worktree",
      },
      {
        id: "t2",
        projectId: "p1",
        title: "Other worktree",
        status: "todo",
        workspaceMode: "worktree",
      },
      {
        id: "t3",
        projectId: "p1",
        title: "Hidden direct session",
        status: "todo",
        workspaceMode: "project_root",
      },
    ];
    const client = createMockClient(state);
    const Wrapper = createHookWrapper(
      client,
      createTestQueryClient(),
      createChatStore(client.session),
    );
    useWorkspaceSelectionStore.getState().selectTask("t1", "p1");

    render(
      <Wrapper>
        <AppI18nProvider>
          <PlatformProvider adapter={createStubPlatform()}>
            <TooltipProvider>
              <WorkspaceView userName="Eric" />
            </TooltipProvider>
          </PlatformProvider>
        </AppI18nProvider>
      </Wrapper>,
    );

    await user.click(
      await screen.findByRole("button", { name: /选择分支|Select branch/ }),
    );

    expect(screen.getByText("Other worktree")).toBeInTheDocument();
    expect(screen.queryByText("Hidden direct session")).toBeNull();
    expect(screen.queryByText(/^直聊$|^Direct chat$/)).toBeNull();
    expect(
      screen.getByText(/创建并检出新分支|Create and checkout a new branch/),
    ).toBeInTheDocument();
  });

  it("creates a project-root task and session behind an optimistic first message", async () => {
    const user = userEvent.setup();
    const state = createMockClientState();
    state.projects = [{ id: "p1", name: "Ora", rootPath: "/ora" }];
    const baseClient = createMockClient(state);
    const calls: string[] = [];
    let finishTaskCreate!: () => void;
    const taskCreateGate = new Promise<void>((resolve) => {
      finishTaskCreate = resolve;
    });
    const client: ContractsClient = {
      ...baseClient,
      task: {
        ...baseClient.task,
        create: async (request, options) => {
          calls.push("task");
          await taskCreateGate;
          return baseClient.task.create(request, options);
        },
      },
      session: {
        ...baseClient.session,
        create: async (request, options) => {
          calls.push("session");
          return baseClient.session.create(request, options);
        },
        prompt: async function* (request, options) {
          calls.push("prompt");
          yield* baseClient.session.prompt(request, options);
        },
      },
    };
    const chatStore = createChatStore(client.session);
    const Wrapper = createHookWrapper(
      client,
      createTestQueryClient(),
      chatStore,
    );
    useWorkspaceSelectionStore.getState().selectProject("p1");

    render(
      <Wrapper>
        <AppI18nProvider>
          <PlatformProvider adapter={createStubPlatform()}>
            <TooltipProvider>
              <WorkspaceView userName="Eric" />
            </TooltipProvider>
          </PlatformProvider>
        </AppI18nProvider>
      </Wrapper>,
    );

    const composer = await screen.findByRole("textbox");
    await waitFor(() => expect(composer).toBeEnabled());
    expect(screen.queryByText(/直聊|Direct chat/)).toBeNull();
    const message = "你好   workspace mode";
    await user.type(composer, message);
    await user.click(
      screen.getByRole("button", { name: /发送消息|Send message/ }),
    );

    await waitFor(() => {
      expect(useWorkspaceSelectionStore.getState().selection.projectId).toBe(
        "p1",
      );
      expect(useWorkspaceSelectionStore.getState().selection.taskId).toBeNull();
      expect(useWorkspaceSelectionStore.getState().selection.sessionId).toMatch(
        /^draft-/,
      );
    });
    expect(screen.getByText(/你好\s+workspace mode/)).toBeInTheDocument();
    expect(state.tasks).toEqual([]);

    finishTaskCreate();

    await waitFor(() => {
      expect(state.tasks).toEqual([
        {
          id: "t1",
          projectId: "p1",
          title: "你好 workspa",
          status: "todo",
          workspaceMode: "project_root",
        },
      ]);
      expect(state.sessions).toEqual([
        {
          id: "s1",
          taskId: "t1",
          agentCli: "open_code",
          status: "running",
          historyState: { type: "writable" },
        },
      ]);
      expect(calls).toEqual(["task", "session", "prompt"]);
    });
    expect(useWorkspaceSelectionStore.getState().selection).toEqual({
      projectId: "p1",
      taskId: "t1",
      sessionId: "s1",
      workflowRunId: null,
    });
    expect(chatStore.getState().conversations.s1?.isLoaded).toBe(true);
  });

  it("keeps a created direct task when session creation fails and reuses it on retry", async () => {
    const user = userEvent.setup();
    const state = createMockClientState();
    state.projects = [{ id: "p1", name: "Ora", rootPath: "/ora" }];
    const baseClient = createMockClient(state);
    let taskCreateCalls = 0;
    let sessionCreateCalls = 0;
    const client: ContractsClient = {
      ...baseClient,
      task: {
        ...baseClient.task,
        create: async (request, options) => {
          taskCreateCalls += 1;
          return baseClient.task.create(request, options);
        },
      },
      session: {
        ...baseClient.session,
        create: async (request, options) => {
          sessionCreateCalls += 1;
          if (sessionCreateCalls === 1) throw new Error("session unavailable");
          return baseClient.session.create(request, options);
        },
      },
    };
    const chatStore = createChatStore(client.session);
    const Wrapper = createHookWrapper(
      client,
      createTestQueryClient(),
      chatStore,
    );
    useWorkspaceSelectionStore.getState().selectProject("p1");

    render(
      <Wrapper>
        <AppI18nProvider>
          <PlatformProvider adapter={createStubPlatform()}>
            <TooltipProvider>
              <WorkspaceView userName="Eric" />
            </TooltipProvider>
          </PlatformProvider>
        </AppI18nProvider>
      </Wrapper>,
    );

    const composer = await screen.findByRole("textbox");
    await waitFor(() => expect(composer).toBeEnabled());
    await user.type(composer, "first attempt");
    await user.click(
      screen.getByRole("button", { name: /发送消息|Send message/ }),
    );

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "session unavailable",
    );
    expect(state.tasks).toHaveLength(1);
    expect(state.sessions).toHaveLength(0);
    expect(useWorkspaceSelectionStore.getState().selection.taskId).toBe("t1");

    await user.type(screen.getByRole("textbox"), "retry");
    await user.click(
      screen.getByRole("button", { name: /发送消息|Send message/ }),
    );

    await waitFor(() => expect(state.sessions).toHaveLength(1));
    expect(taskCreateCalls).toBe(1);
    expect(sessionCreateCalls).toBe(2);
    expect(state.tasks).toHaveLength(1);
  });

  it("shows task creation failures in the optimistic draft conversation", async () => {
    const user = userEvent.setup();
    const state = createMockClientState();
    state.projects = [{ id: "p1", name: "Ora", rootPath: "/ora" }];
    const baseClient = createMockClient(state);
    const client: ContractsClient = {
      ...baseClient,
      task: {
        ...baseClient.task,
        create: async () => {
          throw new Error("task unavailable");
        },
      },
    };
    const chatStore = createChatStore(client.session);
    const Wrapper = createHookWrapper(
      client,
      createTestQueryClient(),
      chatStore,
    );
    useWorkspaceSelectionStore.getState().selectProject("p1");

    render(
      <Wrapper>
        <AppI18nProvider>
          <PlatformProvider adapter={createStubPlatform()}>
            <TooltipProvider>
              <WorkspaceView userName="Eric" />
            </TooltipProvider>
          </PlatformProvider>
        </AppI18nProvider>
      </Wrapper>,
    );

    const composer = await screen.findByRole("textbox");
    await waitFor(() => expect(composer).toBeEnabled());
    await user.type(composer, "cannot start");
    await user.click(
      screen.getByRole("button", { name: /发送消息|Send message/ }),
    );

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "task unavailable",
    );
    expect(screen.getByText("cannot start")).toBeInTheDocument();
    expect(state.tasks).toEqual([]);
    expect(state.sessions).toEqual([]);
    expect(useWorkspaceSelectionStore.getState().selection.taskId).toBeNull();
    expect(useWorkspaceSelectionStore.getState().selection.sessionId).toMatch(
      /^draft-/,
    );
  });
});

describe("directChatTitle", () => {
  it("normalizes whitespace and takes exactly ten Unicode characters", () => {
    expect(directChatTitle("  你好   workspace mode  ")).toBe("你好 workspa");
  });
});
