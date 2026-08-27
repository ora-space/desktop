import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createChatStore } from "@ora/chat";
import type {
  ContractsClient,
  SwitchSessionAgentRequest,
  WarmSessionResponse,
} from "@ora/contracts";
import { TooltipProvider } from "@ora/ui";
import { PlatformProvider } from "../../platform";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AppI18nProvider } from "../../i18n/i18n";
import { appI18n } from "../../i18n/i18n-instance";
import {
  createHookWrapper,
  createTestQueryClient,
} from "../../test/hook-harness";
import { createStubPlatform } from "../../test/stub-platform";
import {
  createMockClient,
  createMockClientState,
} from "../../test/mock-client";
import { usePendingAgentStore } from "../../state/stores/pending-agent-store";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import { useDraftSessionsStore } from "../../state/stores/draft-sessions-store";
import { useComposerInputStore } from "../../state/stores/composer-input-store";
import { useUiStore } from "../../state/stores/ui-store";
import { startSessionDraft } from "../../state/session-drafts";
import { useAgentModelStore } from "../../state/stores/agent-model-store";
import {
  DEFAULT_SETTINGS,
  useSettingsStore,
} from "../../state/stores/settings-store";
import { WorkspaceView } from "./workspace-view";
import { directChatTitle } from "./workspace-view-utils";

function composerText(element: HTMLElement): string {
  return element.dataset.composerText ?? "";
}

/** Flushes conversation hydrate / chip-inject microtasks scheduled from effects. */
async function flushComposerEffects(): Promise<void> {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

beforeEach(() => {
  useWorkspaceSelectionStore.getState().clearSelection();
  useDraftSessionsStore.getState().clear();
  useComposerInputStore.getState().reset();
  useUiStore.setState({ workflowEditorOpen: false });
  // Outlives a render on purpose — remembering one CLI's models across chat
  // surfaces is the point of the store — so each test has to start from a CLI
  // nothing has handshaken, or an earlier test's list would answer for it.
  useAgentModelStore.setState({ known: {} });
  // Agent picks outlive a render, so a test that leaves one recorded would hand
  // the next one a surface already pointing somewhere it never chose.
  usePendingAgentStore.setState({ selections: {}, switches: {} });
  useSettingsStore.setState({
    settings: { ...DEFAULT_SETTINGS, agentCli: "ora-space.opencode" },
  });
});

describe("WorkspaceView", () => {
  it("reloads a selected running session after the in-memory chat store is recreated", async () => {
    const state = createMockClientState();
    state.projects = [{ id: "p1", name: "Ora" }];
    state.tasks = [
      {
        id: "t1",
        projectId: "p1",
        workspaceId: "workspace-t1",
        title: "Refresh history",
      },
    ];
    state.sessions = [
      {
        id: "s1",
        workspaceId: "workspace-t1",
        agentRef: "ora-space.opencode",
        status: "running",
        title: null,
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

  it("shows the Changes button for a selected task's review panel", async () => {
    const state = createMockClientState();
    state.projects = [{ id: "p1", name: "Ora" }];
    state.tasks = [
      {
        id: "t1",
        projectId: "p1",
        workspaceId: "workspace-t1",
        title: "Worktree task",
      },
    ];
    const client = createMockClient(state);
    const chatStore = createChatStore(client.session);
    const Wrapper = createHookWrapper(
      client,
      createTestQueryClient(),
      chatStore,
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

    const toolbar = await screen.findByRole("group", {
      name: /工作区审查面板|Workspace review panels/,
    });
    expect(
      within(toolbar).getByRole("button", { name: /^变更$|^Changes$/ }),
    ).toBeInTheDocument();
  });

  it("shows the Changes button for a selected project with no task open", async () => {
    const state = createMockClientState();
    state.projects = [{ id: "p1", name: "Ora" }];
    const client = createMockClient(state);
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

    const toolbar = await screen.findByRole("group", {
      name: /工作区审查面板|Workspace review panels/,
    });
    expect(
      within(toolbar).getByRole("button", { name: /^变更$|^Changes$/ }),
    ).toBeInTheDocument();
  });

  it("warns when loaded history contains records whose positions are unknown", async () => {
    const state = createMockClientState();
    state.projects = [{ id: "p1", name: "Ora" }];
    state.tasks = [
      {
        id: "t1",
        projectId: "p1",
        workspaceId: "workspace-t1",
        title: "Damaged history",
      },
    ];
    state.sessions = [
      {
        id: "s1",
        workspaceId: "workspace-t1",
        agentRef: "ora-space.opencode",
        status: "running",
        title: null,
        historyState: { type: "writable" },
      },
    ];
    const client = createMockClient(state);
    client.session.load = async function* () {
      yield {
        type: "history_notice" as const,
        notice: { type: "unreadable_records" as const, count: 2 },
      };
      yield { type: "completed" as const };
    };
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

    const banner = await screen.findByRole("alert");
    expect(banner).toHaveTextContent(
      /有 2 条历史记录无法读取|2 history records could not be read/,
    );
    expect(within(banner).queryByRole("button")).toBeNull();
    expect(await screen.findByRole("textbox")).toBeEnabled();
  });

  it("does not load history for a newly initialized session", async () => {
    const state = createMockClientState();
    state.projects = [{ id: "p1", name: "Ora" }];
    state.tasks = [
      {
        id: "t1",
        projectId: "p1",
        workspaceId: "workspace-p1",
        title: "Direct chat",
      },
    ];
    state.sessions = [
      {
        id: "s1",
        workspaceId: "workspace-p1",
        agentRef: "ora-space.opencode",
        status: "running",
        title: null,
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

    const textbox = await screen.findByRole("textbox");
    expect(textbox).toHaveAttribute("aria-disabled", "true");
    expect(textbox).toHaveAttribute("contenteditable", "false");
  });

  it("keeps agent selection enabled while an untouched chat cannot send", async () => {
    const user = userEvent.setup();
    useSettingsStore.setState({ settings: { ...DEFAULT_SETTINGS } });
    const state = createMockClientState();
    state.projects = [{ id: "p1", name: "Ora" }];
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

    const textbox = await screen.findByRole("textbox");
    expect(textbox).toHaveAttribute("aria-disabled", "true");
    const modelSelector = screen.getByRole("button", {
      name: /选择模型|Select model/,
    });
    expect(modelSelector).toBeEnabled();

    await user.click(modelSelector);
    await user.click(await screen.findByText("OpenCode"));

    await waitFor(() => expect(textbox).toBeEnabled());
    expect(useSettingsStore.getState().settings.agentCli).toBe(
      "ora-space.opencode",
    );
  });

  it("does not repeat the default direct-chat mode in the composer context", async () => {
    const state = createMockClientState();
    state.projects = [{ id: "p1", name: "Ora" }];
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
    state.projects = [{ id: "p1", name: "Ora" }];
    state.tasks = [
      {
        id: "t1",
        projectId: "p1",
        workspaceId: "workspace-t1",
        title: "Current worktree",
      },
      {
        id: "t2",
        projectId: "p1",
        workspaceId: "workspace-t2",
        title: "Other worktree",
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
    expect(screen.queryByText(/^直聊$|^Direct chat$/)).toBeNull();
    expect(
      screen.getByText(/创建并检出新分支|Create and checkout a new branch/),
    ).toBeInTheDocument();
  });

  it("sends an ordinary chat through the project's main workspace", async () => {
    const user = userEvent.setup();
    const state = createMockClientState();
    state.projects = [{ id: "p1", name: "Ora" }];
    const baseClient = createMockClient(state);
    const calls: string[] = [];
    const client: ContractsClient = {
      ...baseClient,
      session: {
        ...baseClient.session,
        attach: async (request, options) => {
          calls.push("attach");
          return baseClient.session.attach(request, options);
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
      expect(useWorkspaceSelectionStore.getState().selection).toEqual({
        projectId: "p1",
        taskId: null,
        sessionId: "s1",
        workflowRunId: null,
        draftId: null,
      });
    });
    expect(screen.getByText(/你好\s+workspace mode/)).toBeInTheDocument();
    expect(state.tasks).toEqual([]);
    await waitFor(() => {
      expect(state.sessions).toEqual([
        {
          id: "s1",
          workspaceId: "workspace-p1",
          agentRef: "ora-space.opencode",
          status: "running",
          title: null,
          historyState: { type: "writable" },
        },
      ]);
      expect(calls).toEqual(["attach", "prompt"]);
    });
    expect(useWorkspaceSelectionStore.getState().selection).toEqual({
      projectId: "p1",
      taskId: null,
      sessionId: "s1",
      workflowRunId: null,
      draftId: null,
    });
    expect(chatStore.getState().conversations.s1?.isLoaded).toBe(true);
  });

  it("retries an ordinary chat with a fresh main-workspace session after attach fails", async () => {
    const user = userEvent.setup();
    const state = createMockClientState();
    state.projects = [{ id: "p1", name: "Ora" }];
    const baseClient = createMockClient(state);
    let attachCalls = 0;
    const client: ContractsClient = {
      ...baseClient,
      session: {
        ...baseClient.session,
        attach: async (request, options) => {
          attachCalls += 1;
          if (attachCalls === 1) throw new Error("session unavailable");
          return baseClient.session.attach(request, options);
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
    expect(state.tasks).toHaveLength(0);
    expect(state.sessions).toHaveLength(0);
    expect(useWorkspaceSelectionStore.getState().selection.taskId).toBeNull();

    await user.type(screen.getByRole("textbox"), "retry");
    await user.click(
      screen.getByRole("button", { name: /发送消息|Send message/ }),
    );

    await waitFor(() => expect(state.sessions).toHaveLength(1));
    expect(attachCalls).toBe(2);
    expect(state.tasks).toHaveLength(0);
  });

  it("unbinds a draft and restores dismissibility when attach fails after bind", async () => {
    const user = userEvent.setup();
    const state = createMockClientState();
    state.projects = [{ id: "p1", name: "Ora" }];
    const baseClient = createMockClient(state);
    let attachCalls = 0;
    const client: ContractsClient = {
      ...baseClient,
      session: {
        ...baseClient.session,
        attach: async (request, options) => {
          attachCalls += 1;
          if (attachCalls === 1) throw new Error("session unavailable");
          return baseClient.session.attach(request, options);
        },
      },
    };
    const chatStore = createChatStore(client.session);
    const Wrapper = createHookWrapper(
      client,
      createTestQueryClient(),
      chatStore,
    );
    const draftId = startSessionDraft({ projectId: "p1", taskId: null });

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
    await user.type(composer, "from draft");
    await user.click(
      screen.getByRole("button", { name: /发送消息|Send message/ }),
    );

    await waitFor(() => {
      const draft = useDraftSessionsStore
        .getState()
        .drafts.find((candidate) => candidate.id === draftId);
      expect(draft?.pendingSessionId).toBeNull();
      expect(useWorkspaceSelectionStore.getState().selection.draftId).toBe(
        draftId,
      );
    });
    await waitFor(() => expect(composerText(composer)).toBe("from draft"));
    expect(attachCalls).toBe(1);
    expect(state.sessions).toHaveLength(0);
  });

  it("clears sendInFlight and restores the draft when synchronous send setup fails", async () => {
    const user = userEvent.setup();
    const state = createMockClientState();
    state.projects = [{ id: "p1", name: "Ora" }];
    const client = createMockClient(state);
    const chatStore = createChatStore(client.session);
    const Wrapper = createHookWrapper(
      client,
      createTestQueryClient(),
      chatStore,
    );
    const draftId = startSessionDraft({ projectId: "p1", taskId: null });
    const rekey = vi
      .spyOn(useComposerInputStore.getState(), "rekey")
      .mockImplementationOnce(() => {
        throw new Error("rekey failed");
      });

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
    await user.type(composer, "restore after setup");
    await user.click(
      screen.getByRole("button", { name: /发送消息|Send message/ }),
    );

    await waitFor(() => {
      const draft = useDraftSessionsStore
        .getState()
        .drafts.find((candidate) => candidate.id === draftId);
      expect(draft).toEqual(
        expect.objectContaining({
          pendingSessionId: null,
          sendInFlight: false,
          text: "restore after setup",
        }),
      );
      expect(useWorkspaceSelectionStore.getState().selection.draftId).toBe(
        draftId,
      );
    });
    await waitFor(() =>
      expect(composerText(composer)).toBe("restore after setup"),
    );
    expect(state.sessions).toHaveLength(0);
    rekey.mockRestore();
  });

  it("keeps the persisted session selected when prompt fails after attach", async () => {
    const user = userEvent.setup();
    const state = createMockClientState();
    state.projects = [{ id: "p1", name: "Ora" }];
    const baseClient = createMockClient(state);
    const client: ContractsClient = {
      ...baseClient,
      session: {
        ...baseClient.session,
        prompt: async function* () {
          yield* [];
          throw new Error("provider disconnected");
        },
      },
    };
    const chatStore = createChatStore(client.session);
    const Wrapper = createHookWrapper(
      client,
      createTestQueryClient(),
      chatStore,
    );
    const draftId = startSessionDraft({ projectId: "p1", taskId: null });

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
    await user.type(composer, "after attach");
    await user.click(
      screen.getByRole("button", { name: /发送消息|Send message/ }),
    );

    await waitFor(() => expect(state.sessions).toHaveLength(1));
    await waitFor(() =>
      expect(useWorkspaceSelectionStore.getState().selection).toEqual(
        expect.objectContaining({
          sessionId: state.sessions[0]?.id,
          draftId: null,
        }),
      ),
    );
    // recoverFailedDraftSend must not run after attach: stay on the session and
    // drop the muted row immediately rather than unbound-retrying it.
    expect(
      useDraftSessionsStore.getState().drafts.find((d) => d.id === draftId),
    ).toBeUndefined();
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "provider disconnected",
    );
  });

  it("clears pending send when leaving a draft mid-handshake so return is not stuck", async () => {
    const user = userEvent.setup();
    const state = createMockClientState();
    state.projects = [{ id: "p1", name: "Ora" }];
    state.tasks = [
      {
        id: "t1",
        projectId: "p1",
        workspaceId: "workspace-t1",
        title: "Existing",
      },
    ];
    state.sessions = [
      {
        id: "s-other",
        workspaceId: "workspace-t1",
        agentRef: "ora-space.opencode",
        status: "running",
        title: "Other",
        historyState: { type: "writable" },
      },
    ];
    const baseClient = createMockClient(state);
    let releaseWarm!: () => void;
    const warmGate = new Promise<void>((resolve) => {
      releaseWarm = resolve;
    });
    const client: ContractsClient = {
      ...baseClient,
      session: {
        ...baseClient.session,
        warm: async (request, options) => {
          await warmGate;
          return baseClient.session.warm(request, options);
        },
      },
    };
    const chatStore = createChatStore(client.session);
    const Wrapper = createHookWrapper(
      client,
      createTestQueryClient(),
      chatStore,
    );
    const draftId = startSessionDraft({ projectId: "p1", taskId: null });

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
    await user.type(composer, "leave mid send");
    await user.click(
      screen.getByRole("button", { name: /发送消息|Send message/ }),
    );
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: /正在启动|Starting/i }),
      ).toBeInTheDocument(),
    );

    await act(() => {
      useWorkspaceSelectionStore
        .getState()
        .selectSession("s-other", "t1", "p1");
    });
    await flushComposerEffects();
    // Warm resolve continues through Promise chains after release; flush a
    // macrotask so abandon repark / pendingSend clear stay inside act.
    await act(async () => {
      releaseWarm();
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    await flushComposerEffects();

    await waitFor(() =>
      expect(
        useDraftSessionsStore.getState().drafts.find((d) => d.id === draftId)
          ?.text,
      ).toBe("leave mid send"),
    );

    await act(() => {
      useWorkspaceSelectionStore.getState().selectDraft(draftId, null, "p1");
    });
    await flushComposerEffects();
    await waitFor(() => expect(composerText(composer)).toBe("leave mid send"));
    // Returning must not resurrect a forever-streaming pending turn.
    expect(
      screen.getByRole("button", { name: /发送消息|Send message/ }),
    ).toBeEnabled();
  });

  it("warms a fresh session for retry after attach fails, instead of reusing the consumed id", async () => {
    const user = userEvent.setup();
    const state = createMockClientState();
    state.projects = [{ id: "p1", name: "Ora" }];
    // An already-existing task keeps the warm-session query keyed the same
    // way across both attempts (no task-creation side effect can re-target
    // it), so this isolates the retry behavior this fix is about.
    state.tasks = [
      {
        id: "t1",
        projectId: "p1",
        workspaceId: "workspace-t1",
        title: "Existing task",
      },
    ];
    const baseClient = createMockClient(state);
    let attachCalls = 0;
    const attachedSessionIds: string[] = [];
    const client: ContractsClient = {
      ...baseClient,
      session: {
        ...baseClient.session,
        attach: async (request, options) => {
          attachCalls += 1;
          attachedSessionIds.push(request.sessionId);
          // The real backend's warm pool discards its entry the moment an
          // attach is attempted, whether or not the rest of attach succeeds.
          // Reusing the same warm session id here would incorrectly pass even
          // if the client kept retrying with a stale id.
          if (attachCalls === 1) throw new Error("session unavailable");
          return baseClient.session.attach(request, options);
        },
      },
    };
    const chatStore = createChatStore(client.session);
    const Wrapper = createHookWrapper(
      client,
      createTestQueryClient(),
      chatStore,
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

    const composer = await screen.findByRole("textbox");
    await waitFor(() => expect(composer).toBeEnabled());
    await user.type(composer, "first attempt");
    await user.click(
      screen.getByRole("button", { name: /发送消息|Send message/ }),
    );

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "session unavailable",
    );

    await user.type(screen.getByRole("textbox"), "retry");
    await user.click(
      screen.getByRole("button", { name: /发送消息|Send message/ }),
    );

    await waitFor(() => expect(state.sessions).toHaveLength(1));
    expect(attachCalls).toBe(2);
    // The retry must not reuse the id the failed attach already consumed.
    expect(attachedSessionIds[1]).not.toBe(attachedSessionIds[0]);
    expect(state.sessions[0]?.id).toBe(attachedSessionIds[1]);
  });

  it("shows a model switch that never reached the agent", async () => {
    const user = userEvent.setup();
    const state = createMockClientState();
    state.projects = [{ id: "p1", name: "Ora" }];
    const baseClient = createMockClient(state);
    const client: ContractsClient = {
      ...baseClient,
      session: {
        ...baseClient.session,
        setConfig: async () => {
          throw new Error("agent unreachable");
        },
      },
    };
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

    // The warm session supplies the picker's models, so wait for them to land.
    const picker = await screen.findByRole("button", {
      name: /选择模型|Select model/,
    });
    await waitFor(() => expect(picker).toHaveTextContent("Big Pickle"));
    await user.click(picker);
    await user.click(
      await screen.findByRole("menuitem", { name: "Small Pickle" }),
    );

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "agent unreachable",
    );
    // The rejected switch must not be shown as if it took effect.
    expect(picker).toHaveTextContent("Big Pickle");
  });

  it("keeps a switched model after the surface remounts with a still-warm session", async () => {
    const user = userEvent.setup();
    const state = createMockClientState();
    state.projects = [{ id: "p1", name: "Ora" }];
    const baseClient = createMockClient(state);
    const warm = vi.fn(baseClient.session.warm);
    const client: ContractsClient = {
      ...baseClient,
      session: {
        ...baseClient.session,
        warm,
        // Reports back whichever model was requested, the way an agent answers
        // a switch it accepted �?the mock's default ignores the request.
        setConfig: async (req) => ({
          configOptions: state.configOptions.map((option) =>
            option.type === "select"
              ? { ...option, currentValue: req.value }
              : option,
          ),
        }),
      },
    };
    // One query client and one chat store across both renders, so this is the
    // same app session leaving a surface and coming back to it.
    const Wrapper = createHookWrapper(
      client,
      createTestQueryClient(),
      createChatStore(client.session),
    );
    useWorkspaceSelectionStore.getState().selectProject("p1");

    const renderView = () =>
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

    const first = renderView();
    let picker = await screen.findByRole("button", {
      name: /选择模型|Select model/,
    });
    await waitFor(() => expect(picker).toHaveTextContent("Big Pickle"));
    await user.click(picker);
    await user.click(
      await screen.findByRole("menuitem", { name: "Small Pickle" }),
    );
    await waitFor(() => expect(picker).toHaveTextContent("Small Pickle"));

    first.unmount();
    renderView();

    picker = await screen.findByRole("button", {
      name: /选择模型|Select model/,
    });
    // The warm session is reused rather than re-opened, so its pinned handshake
    // response �?which still names the opening model �?is what a remount sees.
    // Replaying it would silently undo a switch the agent already accepted.
    await waitFor(() => expect(picker).toHaveTextContent("Small Pickle"));
    expect(warm).toHaveBeenCalledOnce();
  });

  it("says the model list is still arriving while the session is being warmed", async () => {
    const user = userEvent.setup();
    const state = createMockClientState();
    state.projects = [{ id: "p1", name: "Ora" }];
    const baseClient = createMockClient(state);
    let openHandshake: (response: WarmSessionResponse) => void = () => {};
    const client: ContractsClient = {
      ...baseClient,
      session: {
        ...baseClient.session,
        // Held open so the picker can be inspected mid-handshake, which is what
        // a real agent's a second or so of start-up looks like.
        warm: () =>
          new Promise<WarmSessionResponse>((resolve) => {
            openHandshake = resolve;
          }),
      },
    };
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

    await user.click(
      await screen.findByRole("button", { name: /选择模型|Select model/ }),
    );
    const menu = await screen.findByRole("menu");
    expect(within(menu).getByText(/加载中|Loading/)).toBeInTheDocument();
    // Announcing "no models" here would be a definite answer to a question the
    // agent has not been asked yet.
    expect(
      within(menu).queryByText(/未提供可选模型|offers no model choice/),
    ).toBeNull();

    openHandshake({
      sessionId: "s1",
      workspaceId: "workspace-p1",
      configOptions: state.configOptions,
    });

    expect(
      await screen.findByRole("menuitem", { name: "Small Pickle" }),
    ).toBeInTheDocument();
  });

  it("offers a CLI's remembered models while a later surface is still warming", async () => {
    const state = createMockClientState();
    state.projects = [
      { id: "p1", name: "Ora" },
      { id: "p2", name: "Ora Docs" },
    ];
    const baseClient = createMockClient(state);
    let handshakes = 0;
    const client: ContractsClient = {
      ...baseClient,
      session: {
        ...baseClient.session,
        // The first surface handshakes normally; the second is held open, which
        // is the wait this whole cache exists to hide.
        warm: (request) => {
          handshakes += 1;
          if (handshakes === 1) return baseClient.session.warm(request);
          return new Promise<WarmSessionResponse>(() => {});
        },
      },
    };
    // One query client and chat store across both renders, so the second
    // surface opens in the same app run that already learned this CLI's models.
    const Wrapper = createHookWrapper(
      client,
      createTestQueryClient(),
      createChatStore(client.session),
    );

    const renderView = () =>
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

    useWorkspaceSelectionStore.getState().selectProject("p1");
    const first = renderView();
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: /选择模型|Select model/ }),
      ).toHaveTextContent("Big Pickle"),
    );
    first.unmount();

    useWorkspaceSelectionStore.getState().selectProject("p2");
    renderView();

    // A different surface, so nothing about the first one's session is reused —
    // only what its handshake reported the CLI offers, which is what lets this
    // one name a model before its own handshake has answered anything.
    const picker = await screen.findByRole("button", {
      name: /选择模型|Select model/,
    });
    await waitFor(() => expect(picker).toHaveTextContent("Big Pickle"));
    expect(picker).not.toHaveTextContent(/加载中|Loading/);
    expect(handshakes).toBe(2);
  });

  it("offers remembered models to read but not to pick until a session exists", async () => {
    const user = userEvent.setup();
    const state = createMockClientState();
    state.projects = [{ id: "p1", name: "Ora" }];
    const baseClient = createMockClient(state);
    let openHandshake: (response: WarmSessionResponse) => void = () => {};
    const setConfig = vi.fn(baseClient.session.setConfig);
    const client: ContractsClient = {
      ...baseClient,
      session: {
        ...baseClient.session,
        setConfig,
        warm: () =>
          new Promise<WarmSessionResponse>((resolve) => {
            openHandshake = resolve;
          }),
      },
    };
    // Stands in for an earlier surface in this run having handshaken this CLI.
    useAgentModelStore.setState({
      known: { "ora-space.opencode": state.configOptions },
    });
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

    await user.click(
      await screen.findByRole("button", { name: /选择模型|Select model/ }),
    );
    // The remembered list answers what the user opened the menu to find out,
    // so it is shown rather than withheld behind the handshake.
    const item = await screen.findByRole("menuitem", { name: "Small Pickle" });
    // Applying a choice needs a session id, and there is none until the
    // handshake lands — so the value is offered to read, not to act on.
    expect(item).toHaveAttribute("data-disabled");
    await user.click(item);
    expect(setConfig).not.toHaveBeenCalled();

    openHandshake({
      sessionId: "s1",
      workspaceId: "workspace-p1",
      configOptions: state.configOptions,
    });

    await waitFor(() =>
      expect(
        screen.getByRole("menuitem", { name: "Small Pickle" }),
      ).not.toHaveAttribute("data-disabled"),
    );
  });

  it("says the model list is still arriving while a selected session replays", async () => {
    const user = userEvent.setup();
    const state = createMockClientState();
    state.projects = [{ id: "p1", name: "Ora" }];
    state.tasks = [
      {
        id: "t1",
        projectId: "p1",
        workspaceId: "workspace-t1",
        title: "Replaying",
      },
    ];
    state.sessions = [
      {
        id: "s1",
        workspaceId: "workspace-t1",
        agentRef: "ora-space.opencode",
        status: "running",
        title: null,
        historyState: { type: "writable" },
      },
    ];
    const client = createMockClient(state);
    let finishReplay: () => void = () => {};
    const replayed = new Promise<void>((resolve) => {
      finishReplay = resolve;
    });
    // A selected session gets its options from `session/load`, which reports
    // them partway through the stream rather than at its start.
    client.session.load = async function* () {
      await replayed;
      yield {
        type: "session_update" as const,
        update: {
          sessionUpdate: "config_option_update" as const,
          configOptions: state.configOptions,
        },
      };
      yield { type: "completed" as const };
    };
    const Wrapper = createHookWrapper(
      client,
      createTestQueryClient(),
      createChatStore(client.session),
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

    await user.click(
      await screen.findByRole("button", { name: /选择模型|Select model/ }),
    );
    const menu = await screen.findByRole("menu");
    // The replay seeds an empty option set before the agent reports the real
    // one, which must not be mistaken for a settled "no models" answer.
    await waitFor(() =>
      expect(within(menu).getByText(/加载中|Loading/)).toBeInTheDocument(),
    );
    expect(
      within(menu).queryByText(/未提供可选模型|offers no model choice/),
    ).toBeNull();

    finishReplay();

    expect(
      await screen.findByRole("menuitem", { name: "Small Pickle" }),
    ).toBeInTheDocument();
  });

  it("still reports an agent that offers no model choice", async () => {
    const user = userEvent.setup();
    const state = createMockClientState();
    state.projects = [{ id: "p1", name: "Ora" }];
    state.configOptions = [];
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

    await user.click(
      await screen.findByRole("button", { name: /选择模型|Select model/ }),
    );
    const menu = await screen.findByRole("menu");

    await waitFor(() =>
      expect(
        within(menu).getByText(/未提供可选模型|offers no model choice/),
      ).toBeInTheDocument(),
    );
    expect(within(menu).queryByText(/加载中|Loading/)).toBeNull();
  });

  /**
   * Builds a client whose `warm` reports Claude's own models, so a switch can be
   * observed offering the incoming CLI's list rather than the outgoing one's.
   */
  function createSwitchTargetClient(
    state: ReturnType<typeof createMockClientState>,
  ) {
    const baseClient = createMockClient(state);
    const switched: SwitchSessionAgentRequest[] = [];
    const prompted: string[] = [];
    const client: ContractsClient = {
      ...baseClient,
      session: {
        ...baseClient.session,
        prompt: (request, options) => {
          prompted.push(request.sessionId);
          return baseClient.session.prompt(request, options);
        },
        warm: async (request, options) => {
          const response = await baseClient.session.warm(request, options);
          if (request.agentRef !== "ora-space.claude") return response;
          return {
            ...response,
            configOptions: [
              {
                id: "model",
                name: "Model",
                category: "model",
                type: "select",
                currentValue: "claude/sonnet",
                options: [
                  { value: "claude/sonnet", name: "Sonnet" },
                  { value: "claude/haiku", name: "Haiku" },
                ],
              },
            ],
          };
        },
        switchAgent: async (request, options) => {
          switched.push(request);
          return baseClient.session.switchAgent(request, options);
        },
      },
    };
    return { client, switched, prompted };
  }

  /** Seeds one running session on OpenCode under a worktree task. */
  function seedSwitchableSession(
    state: ReturnType<typeof createMockClientState>,
  ) {
    state.projects = [{ id: "p1", name: "Ora" }];
    state.tasks = [
      {
        id: "t1",
        projectId: "p1",
        workspaceId: "workspace-t1",
        title: "Switch agent",
      },
    ];
    state.sessions = [
      {
        id: "s1",
        workspaceId: "workspace-t1",
        agentRef: "ora-space.opencode",
        status: "running",
        title: null,
        historyState: { type: "writable" },
      },
    ];
  }

  it("offers the incoming agent's models without rebinding the session yet", async () => {
    const user = userEvent.setup();
    const state = createMockClientState();
    seedSwitchableSession(state);
    const { client, switched } = createSwitchTargetClient(state);
    const Wrapper = createHookWrapper(
      client,
      createTestQueryClient(),
      createChatStore(client.session),
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

    await user.click(
      await screen.findByRole("button", { name: /选择模型|Select model/ }),
    );
    await user.click(
      await screen.findByRole("menuitem", { name: "Claude Code" }),
    );

    // Picking a CLI is only half the decision, so the menu is still open on the
    // models that CLI actually offers rather than the ones it replaced. Those
    // come from warming it, which leaves the conversation's own agent running.
    const menu = await screen.findByRole("menu");
    expect(
      await within(menu).findByRole("menuitem", { name: "Haiku" }),
    ).toBeInTheDocument();
    expect(
      within(menu).queryByRole("menuitem", { name: "Small Pickle" }),
    ).toBeNull();
    // Rebinding here would tear down an agent that may be mid-reply, so nothing
    // is asked of the backend until the next message carries the move.
    expect(switched).toEqual([]);
    expect(state.sessions[0]?.agentRef).toBe("ora-space.opencode");
  });

  it("commits a recorded agent move with the next message", async () => {
    const user = userEvent.setup();
    const state = createMockClientState();
    seedSwitchableSession(state);
    const { client, switched } = createSwitchTargetClient(state);
    const Wrapper = createHookWrapper(
      client,
      createTestQueryClient(),
      createChatStore(client.session),
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
    await user.click(
      await screen.findByRole("button", { name: /选择模型|Select model/ }),
    );
    await user.click(
      await screen.findByRole("menuitem", { name: "Claude Code" }),
    );
    await user.keyboard("{Escape}");

    await user.type(await screen.findByRole("textbox"), "hello");
    await user.keyboard("{Enter}");

    await waitFor(() =>
      expect(state.sessions[0]?.agentRef).toBe("ora-space.claude"),
    );
    expect(switched).toEqual([
      { sessionId: "s1", agentRef: "ora-space.claude" },
    ]);
  });

  it("sends without rebinding when the picker returns to the session's own agent", async () => {
    const user = userEvent.setup();
    const state = createMockClientState();
    seedSwitchableSession(state);
    const { client, switched, prompted } = createSwitchTargetClient(state);
    const Wrapper = createHookWrapper(
      client,
      createTestQueryClient(),
      createChatStore(client.session),
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
    await user.click(
      await screen.findByRole("button", { name: /选择模型|Select model/ }),
    );
    await user.click(
      await screen.findByRole("menuitem", { name: "Claude Code" }),
    );
    // Back to the CLI the conversation was already running on. Nothing was
    // rebound in between, so this is a withdrawn move rather than a second one.
    await user.click(await screen.findByRole("menuitem", { name: "OpenCode" }));
    await user.keyboard("{Escape}");

    await user.type(await screen.findByRole("textbox"), "hello");
    await user.keyboard("{Enter}");

    await waitFor(() => expect(prompted).toEqual(["s1"]));
    // Asking the backend to move a session onto its own agent is refused with
    // `session_agent_unchanged`, which would have failed the message with it.
    expect(switched).toEqual([]);
    expect(state.sessions[0]?.agentRef).toBe("ora-space.opencode");
  });

  it("resumes a session whose history stopped recording", async () => {
    const user = userEvent.setup();
    const state = createMockClientState();
    state.projects = [{ id: "p1", name: "Ora" }];
    state.tasks = [
      {
        id: "t1",
        projectId: "p1",
        workspaceId: "workspace-t1",
        title: "Broken history",
      },
    ];
    state.sessions = [
      {
        id: "s1",
        workspaceId: "workspace-t1",
        agentRef: "ora-space.opencode",
        status: "running",
        title: null,
        historyState: { type: "degraded", reason: "no space left on device" },
      },
    ];
    const client = createMockClient(state);
    const Wrapper = createHookWrapper(
      client,
      createTestQueryClient(),
      createChatStore(client.session),
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

    const banner = await screen.findByRole("alert");
    expect(banner).toHaveTextContent("no space left on device");
    await user.click(
      within(banner).getByRole("button", { name: /恢复记录|Resume history/ }),
    );

    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
    expect(state.sessions[0]?.historyState).toEqual({ type: "writable" });
  });

  it("renders the workflow editor in place of chat when the editor is open", async () => {
    await act(() => appI18n.changeLanguage("zh-CN"));
    const state = createMockClientState();
    state.projects = [{ id: "p1", name: "Ora" }];
    const client = createMockClient(state);
    const Wrapper = createHookWrapper(
      client,
      createTestQueryClient(),
      createChatStore(client.session),
    );
    useUiStore.setState({ workflowEditorOpen: true });

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

    expect(
      await screen.findByRole("button", {
        name: /导出工作流|Export workflow/,
      }),
    ).toBeInTheDocument();
    expect(
      screen.queryByLabelText(/给 Ora 的消息|Message to Ora/),
    ).not.toBeInTheDocument();
  });
});

describe("directChatTitle", () => {
  it("normalizes whitespace and takes exactly ten Unicode characters", () => {
    expect(directChatTitle("  你好   workspace mode  ")).toBe("你好 workspa");
  });
});
