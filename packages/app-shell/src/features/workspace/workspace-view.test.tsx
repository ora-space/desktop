import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createChatStore } from "@ora/chat";
import type {
  ContractsClient,
  SwitchSessionAgentRequest,
  ListAgentModelsResponse,
  StartSessionRequest,
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
  createTestClient,
  type TestHandlers,
} from "../../test/contracts-transport";
import {
  createWorkspaceMemory,
  workspaceHandlers,
} from "../../test/memory/workspaces";
import {
  createSessionMemory,
  sessionHandlers,
} from "../../test/memory/sessions";
import {
  createAgentRuntimeMemory,
  agentRuntimeHandlers,
} from "../../test/memory/agent-runtime";
import { createPluginMemory, pluginHandlers } from "../../test/memory/plugins";
import { createAgentMemory, agentHandlers } from "../../test/memory/agents";
import { createSkillMemory, skillHandlers } from "../../test/memory/skills";
import {
  createWorkflowMemory,
  workflowHandlers,
} from "../../test/memory/workflows";
import { readyEffectHandlers } from "../../test/memory/effects";
import { usePendingAgentStore } from "../../state/stores/pending-agent-store";
import { useAgentModelPreferenceStore } from "../../state/stores/agent-model-preference-store";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import { useDraftSessionsStore } from "../../state/stores/draft-sessions-store";
import { useComposerInputStore } from "../../state/stores/composer-input-store";
import { useUiStore } from "../../state/stores/ui-store";
import { startSessionDraft } from "../../state/session-drafts";
import {
  DEFAULT_SETTINGS,
  useSettingsStore,
} from "../../state/stores/settings-store";
import { WorkspaceView } from "./workspace-view";
import { directChatTitle } from "./workspace-view-utils";
import { AGENT_REF } from "../../test/agent-identity";

/** State for this test surface; no unrelated domain fixtures are initialized. */
function createFixtureState() {
  return {
    ...createWorkspaceMemory(),
    ...createSessionMemory(),
    ...createAgentRuntimeMemory(),
    ...createPluginMemory(),
    ...createAgentMemory(),
    ...createSkillMemory(),
    ...createWorkflowMemory(),
  };
}

type FixtureState = ReturnType<typeof createFixtureState>;

/** Explicit domain composition for the behaviors exercised by this test file. */
function createFixtureHandlers(state: FixtureState): TestHandlers {
  return {
    ...workspaceHandlers(state),
    ...sessionHandlers(state),
    ...agentRuntimeHandlers(state),
    ...pluginHandlers(state),
    ...agentHandlers(state),
    ...skillHandlers(state),
    ...workflowHandlers(state),
    ...readyEffectHandlers(),
  };
}

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
  // Agent picks outlive a render, so a test that leaves one recorded would hand
  // the next one a surface already pointing somewhere it never chose.
  usePendingAgentStore.setState({ selections: {}, switches: {}, models: {} });
  // Remembered per agent and persisted, so a preference one test records would
  // otherwise decide which model the next test's first send starts on.
  useAgentModelPreferenceStore.setState({ models: {} });
  useSettingsStore.setState({
    settings: { ...DEFAULT_SETTINGS, agentCli: AGENT_REF.opencode },
  });
});

describe("WorkspaceView", () => {
  it("reloads a selected running session after the in-memory chat store is recreated", async () => {
    const state = createFixtureState();
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
        agentRef: AGENT_REF.opencode,
        status: "running",
        title: null,
        historyState: { type: "writable" },
      },
    ];
    const clientHandlers: TestHandlers = createFixtureHandlers(state);
    const client = createTestClient(clientHandlers);
    const load = vi.fn(async function* () {
      yield { type: "completed" as const };
    });
    clientHandlers.loadSession = load;
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

  it("shows reported usage in the conversation header but hides reload and waiting states", async () => {
    const state = createFixtureState();
    state.projects = [{ id: "p1", name: "Ora" }];
    state.tasks = [
      {
        id: "t1",
        projectId: "p1",
        workspaceId: "workspace-t1",
        title: "Usage header",
      },
    ];
    state.sessions = [
      {
        id: "s1",
        workspaceId: "workspace-t1",
        agentRef: AGENT_REF.opencode,
        status: "running",
        title: null,
        historyState: { type: "writable" },
      },
    ];
    const clientHandlers: TestHandlers = createFixtureHandlers(state);
    const client = createTestClient(clientHandlers);
    clientHandlers.loadSession = vi.fn(async function* () {
      yield { type: "completed" as const };
    });
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

    await waitFor(() =>
      expect(chatStore.getState().conversations.s1?.isLoaded).toBe(true),
    );
    expect(
      screen.queryByRole("button", {
        name: /Usage data is not saved|用量数据不会随历史记录保存/,
      }),
    ).not.toBeInTheDocument();

    act(() => {
      chatStore.setState((current) => {
        const conversation = current.conversations.s1;
        if (!conversation) throw new Error("expected loaded conversation");
        return {
          conversations: {
            ...current.conversations,
            s1: {
              ...conversation,
              usage: {
                context: { status: "awaiting_report" },
                lastTurnTokens: { status: "awaiting_completion" },
              },
            },
          },
        };
      });
    });
    expect(
      screen.queryByRole("button", {
        name: /Waiting for the agent to report usage|正在等待 Agent 上报用量/,
      }),
    ).not.toBeInTheDocument();

    act(() => {
      chatStore.setState((current) => {
        const conversation = current.conversations.s1;
        if (!conversation) throw new Error("expected loaded conversation");
        return {
          conversations: {
            ...current.conversations,
            s1: {
              ...conversation,
              usage: {
                context: {
                  status: "reported",
                  snapshot: {
                    usedTokens: 34_000,
                    sizeTokens: 100_000,
                    receivedAt: Date.now(),
                  },
                },
                lastTurnTokens: { status: "none" },
              },
            },
          },
        };
      });
    });

    const usageButton = screen.getByRole("button", {
      name: /Context 34%|上下文 34%/,
    });
    const locationActions = screen.getByRole("group", {
      name: /Open location|打开位置/,
    });
    const header = locationActions.parentElement;
    expect(header).not.toBeNull();
    expect(header).toContainElement(usageButton);
    expect(
      usageButton.compareDocumentPosition(locationActions) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
  });

  it("shows the Changes button for a selected task's review panel", async () => {
    const state = createFixtureState();
    state.projects = [{ id: "p1", name: "Ora" }];
    state.tasks = [
      {
        id: "t1",
        projectId: "p1",
        workspaceId: "workspace-t1",
        title: "Worktree task",
      },
    ];
    const clientHandlers: TestHandlers = createFixtureHandlers(state);
    const client = createTestClient(clientHandlers);
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
    const state = createFixtureState();
    state.projects = [{ id: "p1", name: "Ora" }];
    const clientHandlers: TestHandlers = createFixtureHandlers(state);
    const client = createTestClient(clientHandlers);
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
    const state = createFixtureState();
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
        agentRef: AGENT_REF.opencode,
        status: "running",
        title: null,
        historyState: { type: "writable" },
      },
    ];
    const clientHandlers: TestHandlers = createFixtureHandlers(state);
    const client = createTestClient(clientHandlers);
    clientHandlers.loadSession = async function* () {
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
    const state = createFixtureState();
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
        agentRef: AGENT_REF.opencode,
        status: "running",
        title: null,
        historyState: { type: "writable" },
      },
    ];
    const clientHandlers: TestHandlers = createFixtureHandlers(state);
    const client = createTestClient(clientHandlers);
    const load = vi.fn(async function* () {
      yield { type: "completed" as const };
    });
    clientHandlers.loadSession = load;
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
    const state = createFixtureState();
    const clientHandlers: TestHandlers = createFixtureHandlers(state);
    const client = createTestClient(clientHandlers);
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
    const state = createFixtureState();
    state.projects = [{ id: "p1", name: "Ora" }];
    const clientHandlers: TestHandlers = createFixtureHandlers(state);
    const client = createTestClient(clientHandlers);
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
      AGENT_REF.opencode,
    );
  });

  it("gates only the send button while the chosen agent is unreachable", async () => {
    const user = userEvent.setup();
    const state = createFixtureState();
    state.projects = [{ id: "p1", name: "Ora" }];
    const entry = state.agentRuntimeStatuses.find(
      (candidate) => candidate.agentRef === AGENT_REF.opencode,
    );
    entry!.status = "unavailable";
    const clientHandlers: TestHandlers = createFixtureHandlers(state);
    const client = createTestClient(clientHandlers);
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
    await user.type(textbox, "hello");
    // With text typed the only reason left to refuse a send is the agent, so
    // the gate — not the empty composer — is what dims the button here.
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: /发送消息|Send message/ }),
      ).toHaveAttribute("aria-disabled", "true"),
    );
    expect(composerText(textbox)).toBe("hello");
    // The state is fixable from the picker, so it must stay actionable.
    expect(
      screen.getByRole("button", { name: /选择模型|Select model/ }),
    ).toBeEnabled();

    await user.hover(
      screen.getByRole("button", { name: /发送消息|Send message/ }),
    );
    expect(
      await screen.findByText(
        /请先选择一个可用的Agent模型|Pick an available agent/,
      ),
    ).toBeVisible();
  });

  it("does not repeat the default direct-chat mode in the composer context", async () => {
    const state = createFixtureState();
    state.projects = [{ id: "p1", name: "Ora" }];
    const clientHandlers: TestHandlers = createFixtureHandlers(state);
    const client = createTestClient(clientHandlers);
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
    expect(
      screen.queryByRole("button", { name: /Spec 模式|Spec mode/ }),
    ).toBeNull();
    expect(screen.queryByText("Spec 模式")).toBeNull();
  });

  it("shows only worktrees in the worktree context menu", async () => {
    const user = userEvent.setup();
    const state = createFixtureState();
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
    const clientHandlers: TestHandlers = createFixtureHandlers(state);
    const client = createTestClient(clientHandlers);
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
    const state = createFixtureState();
    state.projects = [{ id: "p1", name: "Ora" }];
    const baseClientHandlers: TestHandlers = createFixtureHandlers(state);
    const baseClient = createTestClient(baseClientHandlers);
    const calls: string[] = [];
    const client: ContractsClient = createTestClient({
      ...baseClientHandlers,
      startSession: async (request, options) => {
        calls.push("start");
        return baseClient.session.start(request, options);
      },
      promptSession: async function* (request, options) {
        calls.push("prompt");
        yield* baseClient.session.prompt(request, options);
      },
    });
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
          agentRef: AGENT_REF.opencode,
          status: "running",
          title: null,
          historyState: { type: "writable" },
        },
      ]);
      expect(calls).toEqual(["start", "prompt"]);
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
    const state = createFixtureState();
    state.projects = [{ id: "p1", name: "Ora" }];
    const baseClientHandlers: TestHandlers = createFixtureHandlers(state);
    const baseClient = createTestClient(baseClientHandlers);
    let attachCalls = 0;
    const client: ContractsClient = createTestClient({
      ...baseClientHandlers,
      startSession: async (request, options) => {
        attachCalls += 1;
        if (attachCalls === 1) throw new Error("session unavailable");
        return baseClient.session.start(request, options);
      },
    });
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
    const state = createFixtureState();
    state.projects = [{ id: "p1", name: "Ora" }];
    const baseClientHandlers: TestHandlers = createFixtureHandlers(state);
    const baseClient = createTestClient(baseClientHandlers);
    let attachCalls = 0;
    const client: ContractsClient = createTestClient({
      ...baseClientHandlers,
      startSession: async (request, options) => {
        attachCalls += 1;
        if (attachCalls === 1) throw new Error("session unavailable");
        return baseClient.session.start(request, options);
      },
    });
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

  it("keeps the started session when synchronous send setup fails", async () => {
    const user = userEvent.setup();
    const state = createFixtureState();
    state.projects = [{ id: "p1", name: "Ora" }];
    const clientHandlers: TestHandlers = createFixtureHandlers(state);
    const client = createTestClient(clientHandlers);
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

    // `start` persists before any of this setup runs, so the recovery boundary
    // is behind us: restoring the draft would leave a phantom row pointing at a
    // session the backend really created.
    await waitFor(() => expect(state.sessions).toHaveLength(1));
    const sessionId = state.sessions[0]!.id;
    await waitFor(() =>
      expect(useWorkspaceSelectionStore.getState().selection).toEqual(
        expect.objectContaining({ sessionId, draftId: null }),
      ),
    );
    expect(
      useDraftSessionsStore
        .getState()
        .drafts.find((candidate) => candidate.id === draftId),
    ).toBeUndefined();
    rekey.mockRestore();
  });

  it("keeps the persisted session selected when prompt fails after attach", async () => {
    const user = userEvent.setup();
    const state = createFixtureState();
    state.projects = [{ id: "p1", name: "Ora" }];
    const baseClientHandlers: TestHandlers = createFixtureHandlers(state);
    const client: ContractsClient = createTestClient({
      ...baseClientHandlers,
      promptSession: async function* () {
        yield* [];
        throw new Error("provider disconnected");
      },
    });
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
    const state = createFixtureState();
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
        agentRef: AGENT_REF.opencode,
        status: "running",
        title: "Other",
        historyState: { type: "writable" },
      },
    ];
    const baseClientHandlers: TestHandlers = createFixtureHandlers(state);
    const baseClient = createTestClient(baseClientHandlers);
    let releaseStart!: () => void;
    const startGate = new Promise<void>((resolve) => {
      releaseStart = resolve;
    });
    const client: ContractsClient = createTestClient({
      ...baseClientHandlers,
      startSession: async (request, options) => {
        await startGate;
        return baseClient.session.start(request, options);
      },
    });
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
    // The start resolve continues through Promise chains after release; flush a
    // macrotask so abandon repark / pendingSend clear stay inside act.
    await act(async () => {
      releaseStart();
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

  it("creates a fresh session when a retry follows a failed start", async () => {
    const user = userEvent.setup();
    const state = createFixtureState();
    state.projects = [{ id: "p1", name: "Ora" }];
    // An already-existing task keeps both attempts on one chat surface, so no
    // task-creation side effect can re-target the retry.
    state.tasks = [
      {
        id: "t1",
        projectId: "p1",
        workspaceId: "workspace-t1",
        title: "Existing task",
      },
    ];
    const baseClientHandlers: TestHandlers = createFixtureHandlers(state);
    const baseClient = createTestClient(baseClientHandlers);
    let startCalls = 0;
    const startedSessionIds: string[] = [];
    const client: ContractsClient = createTestClient({
      ...baseClientHandlers,
      startSession: async (request, options) => {
        startCalls += 1;
        // A start that fails leaves nothing behind on either side: the backend
        // releases the provider session it had created, and the client holds
        // no identifier it could retry against.
        if (startCalls === 1) throw new Error("session unavailable");
        const response = await baseClient.session.start(request, options);
        startedSessionIds.push(response.session.id);
        return response;
      },
    });
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
    expect(startCalls).toBe(2);
    // Only the retry produced a session, and it is the one now persisted.
    expect(startedSessionIds).toEqual([state.sessions[0]!.id]);
  });

  it("shows a model switch that never reached the agent", async () => {
    const user = userEvent.setup();
    const state = createFixtureState();
    state.projects = [{ id: "p1", name: "Ora" }];
    state.tasks = [
      {
        id: "t1",
        projectId: "p1",
        workspaceId: "workspace-t1",
        title: "Configured",
      },
    ];
    state.sessions = [
      {
        id: "s1",
        workspaceId: "workspace-t1",
        agentRef: AGENT_REF.opencode,
        status: "running",
        title: null,
        historyState: { type: "writable" },
      },
    ];
    const baseClientHandlers: TestHandlers = createFixtureHandlers(state);
    const client: ContractsClient = createTestClient({
      ...baseClientHandlers,
      loadSession: async function* () {
        yield {
          type: "session_update" as const,
          update: {
            sessionUpdate: "config_option_update" as const,
            configOptions: state.configOptions,
          },
        };
        yield { type: "completed" as const };
      },
      setSessionConfig: async () => {
        throw new Error("agent unreachable");
      },
    });
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

  it("keeps a model picked before the first send across a remount", async () => {
    const user = userEvent.setup();
    const state = createFixtureState();
    state.projects = [{ id: "p1", name: "Ora" }];
    const baseClientHandlers: TestHandlers = createFixtureHandlers(state);
    const baseClient = createTestClient(baseClientHandlers);
    const discover = vi.fn(baseClient.agentRuntime.listModels);
    const client: ContractsClient = createTestClient({
      ...baseClientHandlers,
      listAgentModels: discover,
    });
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
    // Nothing was sent, so the pick is still only an intent — one that belongs
    // to this surface and must survive leaving it and coming back.
    await waitFor(() => expect(picker).toHaveTextContent("Small Pickle"));
    // Discovery is a cached query keyed by agent and workspace, so returning to
    // the same surface asks the plugin nothing new.
    expect(discover).toHaveBeenCalledOnce();
  });

  it("says the model list is still arriving while discovery is open", async () => {
    const user = userEvent.setup();
    const state = createFixtureState();
    state.projects = [{ id: "p1", name: "Ora" }];
    const baseClientHandlers: TestHandlers = createFixtureHandlers(state);
    let openHandshake: (response: ListAgentModelsResponse) => void = () => {};
    const client: ContractsClient = createTestClient({
      ...baseClientHandlers,
      listAgentModels: () =>
        new Promise<ListAgentModelsResponse>((resolve) => {
          openHandshake = resolve;
        }),
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
    const menu = await screen.findByRole("menu");
    expect(within(menu).getByText(/加载中|Loading/)).toBeInTheDocument();
    // Announcing "no models" here would be a definite answer to a question the
    // agent has not been asked yet.
    expect(
      within(menu).queryByText(/未提供可选模型|offers no model choice/),
    ).toBeNull();

    openHandshake({
      models: [
        { id: "opencode/big-pickle", displayName: "Big Pickle", default: true },
        {
          id: "opencode/small-pickle",
          displayName: "Small Pickle",
          default: false,
        },
      ],
    });

    expect(
      await screen.findByRole("menuitem", { name: "Small Pickle" }),
    ).toBeInTheDocument();
  });

  it("carries a model picked before the first send into startSession", async () => {
    const user = userEvent.setup();
    const state = createFixtureState();
    state.projects = [{ id: "p1", name: "Ora" }];
    const baseClientHandlers: TestHandlers = createFixtureHandlers(state);
    const baseClient = createTestClient(baseClientHandlers);
    const started: StartSessionRequest[] = [];
    const setConfig = vi.fn(baseClient.session.setConfig);
    const client: ContractsClient = createTestClient({
      ...baseClientHandlers,
      setSessionConfig: setConfig,
      startSession: async (request, options) => {
        started.push(request);
        return baseClient.session.start(request, options);
      },
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
    await user.click(
      await screen.findByRole("menuitem", { name: "Small Pickle" }),
    );
    // No session exists, so there is nothing to configure yet: the choice is
    // recorded locally rather than sent anywhere.
    expect(setConfig).not.toHaveBeenCalled();
    expect(started).toEqual([]);

    await user.keyboard("{Escape}");
    await user.type(await screen.findByRole("textbox"), "hello");
    await user.keyboard("{Enter}");

    // The first send is where that intent is spent, in the same call that
    // creates the session — so the very first turn runs on the chosen model.
    await waitFor(() =>
      expect(started).toEqual([
        {
          workspaceId: "workspace-p1",
          agentRef: AGENT_REF.opencode,
          model: "opencode/small-pickle",
        },
      ]),
    );
    expect(setConfig).not.toHaveBeenCalled();
  });

  it("starts an untouched chat on the model remembered for its agent", async () => {
    const user = userEvent.setup();
    useAgentModelPreferenceStore.setState({
      models: { [AGENT_REF.opencode]: "opencode/small-pickle" },
    });
    const state = createFixtureState();
    state.projects = [{ id: "p1", name: "Ora" }];
    const baseClientHandlers: TestHandlers = createFixtureHandlers(state);
    const baseClient = createTestClient(baseClientHandlers);
    const started: StartSessionRequest[] = [];
    const client: ContractsClient = createTestClient({
      ...baseClientHandlers,
      startSession: async (request, options) => {
        started.push(request);
        return baseClient.session.start(request, options);
      },
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

    // The picker is never opened: what the surface is labelled with has to be
    // what the send asks for, or the first turn silently runs on another model.
    await user.type(await screen.findByRole("textbox"), "hello");
    await user.keyboard("{Enter}");

    await waitFor(() =>
      expect(started).toEqual([
        {
          workspaceId: "workspace-p1",
          agentRef: AGENT_REF.opencode,
          model: "opencode/small-pickle",
        },
      ]),
    );
  });

  it("says the model list is still arriving while a selected session replays", async () => {
    const user = userEvent.setup();
    const state = createFixtureState();
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
        agentRef: AGENT_REF.opencode,
        status: "running",
        title: null,
        historyState: { type: "writable" },
      },
    ];
    const clientHandlers: TestHandlers = createFixtureHandlers(state);
    const client = createTestClient(clientHandlers);
    let finishReplay: () => void = () => {};
    const replayed = new Promise<void>((resolve) => {
      finishReplay = resolve;
    });
    // A selected session gets its options from `session/load`, which reports
    // them partway through the stream rather than at its start.
    clientHandlers.loadSession = async function* () {
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
    const state = createFixtureState();
    state.projects = [{ id: "p1", name: "Ora" }];
    state.configOptions = [];
    const clientHandlers: TestHandlers = createFixtureHandlers(state);
    const client = createTestClient(clientHandlers);
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
   * Builds a client whose discovery reports Claude's own models, so a switch can be
   * observed offering the incoming CLI's list rather than the outgoing one's.
   */
  function createSwitchTargetClient(
    state: ReturnType<typeof createFixtureState>,
  ) {
    const baseClientHandlers: TestHandlers = createFixtureHandlers(state);
    const baseClient = createTestClient(baseClientHandlers);
    const switched: SwitchSessionAgentRequest[] = [];
    const prompted: string[] = [];
    const clientHandlers: TestHandlers = {
      ...baseClientHandlers,
      promptSession: (request, options) => {
        prompted.push(request.sessionId);
        return baseClient.session.prompt(request, options);
      },
      switchSessionAgent: async (request, options) => {
        switched.push(request);
        return baseClient.session.switchAgent(request, options);
      },
      listAgentModels: async (request, options) => {
        const response = await baseClient.agentRuntime.listModels(
          request,
          options,
        );
        if (request.agentRef !== AGENT_REF.claude) return response;
        return {
          ...response,
          models: [
            { id: "claude/sonnet", displayName: "Sonnet", default: true },
            { id: "claude/haiku", displayName: "Haiku", default: false },
          ],
        };
      },
    };
    const client = createTestClient(clientHandlers);
    return { client, handlers: clientHandlers, switched, prompted };
  }

  /** Seeds one running session on OpenCode under a worktree task. */
  function seedSwitchableSession(state: ReturnType<typeof createFixtureState>) {
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
        agentRef: AGENT_REF.opencode,
        status: "running",
        title: null,
        historyState: { type: "writable" },
      },
    ];
  }

  it("offers the incoming agent's models without rebinding the session yet", async () => {
    const user = userEvent.setup();
    const state = createFixtureState();
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
    // come from discovery, which leaves the conversation's own agent running.
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
    expect(state.sessions[0]?.agentRef).toBe(AGENT_REF.opencode);
  });

  it("commits a recorded agent move with the next message", async () => {
    const user = userEvent.setup();
    const state = createFixtureState();
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
      expect(state.sessions[0]?.agentRef).toBe(AGENT_REF.claude),
    );
    expect(switched).toEqual([
      { sessionId: "s1", agentRef: AGENT_REF.claude, model: null },
    ]);
  });

  it("moves a session off an unavailable agent with the next message", async () => {
    const user = userEvent.setup();
    const state = createFixtureState();
    seedSwitchableSession(state);
    state.sessions[0]!.agentRef = "ora-space.opencode";
    state.installedPlugins = state.installedPlugins.filter(
      (plugin) => plugin.id !== AGENT_REF.opencode,
    );
    state.agentRuntimeStatuses = state.agentRuntimeStatuses.filter(
      (status) => status.agentRef !== AGENT_REF.opencode,
    );
    const { client, handlers, switched, prompted } =
      createSwitchTargetClient(state);
    handlers.loadSession = async function* (request) {
      if (request.sessionId === "s1") {
        throw new Error("ora-space.opencode is not installed");
      }
      yield { type: "completed" };
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

    expect(await screen.findByRole("alert")).toHaveAttribute(
      "data-agent-availability",
      "uninstalled",
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

    await waitFor(() => expect(prompted).toEqual(["s1"]));
    expect(switched).toEqual([
      { sessionId: "s1", agentRef: AGENT_REF.claude, model: null },
    ]);
    expect(state.sessions[0]?.agentRef).toBe(AGENT_REF.claude);
  });

  it("sends without rebinding when the picker returns to the session's own agent", async () => {
    const user = userEvent.setup();
    const state = createFixtureState();
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
    expect(state.sessions[0]?.agentRef).toBe(AGENT_REF.opencode);
  });

  it("resumes a session whose history stopped recording", async () => {
    const user = userEvent.setup();
    const state = createFixtureState();
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
        agentRef: AGENT_REF.opencode,
        status: "running",
        title: null,
        historyState: { type: "degraded", reason: "no space left on device" },
      },
    ];
    const clientHandlers: TestHandlers = createFixtureHandlers(state);
    const client = createTestClient(clientHandlers);
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
    const state = createFixtureState();
    state.projects = [{ id: "p1", name: "Ora" }];
    const clientHandlers: TestHandlers = createFixtureHandlers(state);
    const client = createTestClient(clientHandlers);
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
