import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { describe, expect, it, vi } from "vitest";
import type * as acp from "@agentclientprotocol/sdk";
import type { ChatTurn, SessionConversation } from "@ora/chat";
import { createChatStore } from "@ora/chat";
import { RemoteContractError } from "@ora/contracts";
import { toast } from "@ora/ui";
import type {
  GraphWorkflowNodeStatus,
  WorkflowNodeCompletion,
  WorkflowNodeCompletionBlockerKind,
} from "@ora/workflow-runtime";
import { appI18n } from "../../i18n/i18n-instance";
import {
  createTestQueryClient,
  createHookWrapper,
} from "../../test/hook-harness";
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
import {
  createWorkflowRunMemory,
  workflowRunHandlers,
} from "../../test/memory/workflow-runs";
import { RunNodeSessionChat } from "./run-node-session-chat";

/** State for this test surface; no unrelated domain fixtures are initialized. */
function createFixtureState() {
  return {
    ...createWorkspaceMemory(),
    ...createSessionMemory(),
    ...createAgentRuntimeMemory(),
    ...createPluginMemory(),
    ...createAgentMemory(),
    ...createWorkflowRunMemory(),
  };
}

type FixtureState = ReturnType<typeof createFixtureState>;

/** Explicit domain composition for the behaviors exercised by this test file. */
function createFixtureHandlers(state: FixtureState): TestHandlers {
  return {
    getSessionBindings: ({ sessionIds }) => ({
      sessions: sessionIds.map((sessionId) => ({
        type: "not_found" as const,
        sessionId,
      })),
    }),
    ...workspaceHandlers(state),
    ...sessionHandlers(state),
    ...agentRuntimeHandlers(state),
    ...pluginHandlers(state),
    ...agentHandlers(state),
    ...workflowRunHandlers(state),
  };
}

const sessionId = "session-1";
const runId = "run-1";
const nodeId = "node-a";

/** A loaded, quiet conversation so the dock does not stream anything during the test. */
function seededConversation(
  isResponding: boolean,
  turns: ChatTurn[] = [],
  configOptions: acp.SessionConfigOption[] = [],
): SessionConversation {
  return {
    configOptions,
    modelChanges: [],
    historyNotices: [],
    turns,
    availableCommands: [],
    sessionTitle: null,
    sessionUpdatedAt: null,
    isLoaded: true,
    isLoading: false,
    isResponding,
    pendingPermissions: [],
    usage: {
      context: { status: "hidden" },
      lastTurnTokens: { status: "none" },
    },
    error: null,
  };
}

/** A parked turn that settled normally, so the step card is ready to advance. */
function readyCompletion(): WorkflowNodeCompletion {
  return {
    userSeq: 1,
    turnKey: { userSeq: 1, endedSeq: 2 },
    stopReason: "end_turn",
    blockers: [],
    warnings: [],
    outputDigest: "sha256:ready",
    provisional: false,
    ready: true,
  };
}

/** A parked turn with one blocker; `provisional` marks cleanup that has not settled yet. */
function blockedCompletion(
  kind: WorkflowNodeCompletionBlockerKind,
  provisional = false,
): WorkflowNodeCompletion {
  return {
    userSeq: 1,
    blockers: [{ kind, params: {} }],
    warnings: [],
    provisional,
    ready: false,
  };
}

function renderDock(
  status: GraphWorkflowNodeStatus,
  isResponding: boolean,
  turns: ChatTurn[] = [],
  sessionActions?: ReactNode,
  onNodeCompleted?: (nodeId: string) => void,
  configOptions: acp.SessionConfigOption[] = [],
  completion?: WorkflowNodeCompletion,
  nextNodeTitles?: string[],
) {
  const clientHandlers: TestHandlers =
    createFixtureHandlers(createFixtureState());
  const client = createTestClient(clientHandlers);
  const chatStore = createChatStore(client.session);
  chatStore.setState({
    conversations: {
      [sessionId]: seededConversation(isResponding, turns, configOptions),
    },
  });
  render(
    <RunNodeSessionChat
      sessionId={sessionId}
      status={status}
      interaction={{
        runId,
        nodeId,
        ...(completion !== undefined ? { completion } : {}),
        ...(nextNodeTitles !== undefined ? { nextNodeTitles } : {}),
      }}
      sessionActions={sessionActions}
      onNodeCompleted={onNodeCompleted}
    />,
    {
      wrapper: createHookWrapper(client, createTestQueryClient(), chatStore),
    },
  );
  return { client, handlers: clientHandlers, chatStore };
}

/** Renders the same session surface without granting node interaction controls. */
function renderReadOnlyDock() {
  const clientHandlers: TestHandlers =
    createFixtureHandlers(createFixtureState());
  const client = createTestClient(clientHandlers);
  const loadSpy = vi.spyOn(client.session, "load");
  const chatStore = createChatStore(client.session);
  render(
    <RunNodeSessionChat
      sessionId={sessionId}
      status="running"
      sessionActions={<button type="button">返回阶段摘要</button>}
    />,
    {
      wrapper: createHookWrapper(client, createTestQueryClient(), chatStore),
    },
  );
  return loadSpy;
}

describe("RunNodeSessionChat", () => {
  it("forwards the workflow conversation's live diagnostics to the shared panel", async () => {
    const turn: ChatTurn = {
      id: "turn",
      userMessage: {
        kind: "message",
        id: "user",
        role: "user",
        content: "go",
        createdAt: 1,
      },
      items: [
        {
          kind: "message",
          id: "child-message",
          role: "assistant",
          content: "child output",
          createdAt: 1,
          metadata: {
            "opencode/child-session": {
              id: "child",
              parentID: "parent",
              depth: 1,
              title: "Worker",
            },
          },
        },
      ],
      status: "streaming",
      stopReason: null,
      error: null,
      createdAt: 1,
    };
    const { chatStore } = renderDock("running", true, [turn]);
    await act(async () =>
      chatStore.setState((state) => ({
        conversations: {
          ...state.conversations,
          [sessionId]: {
            ...state.conversations[sessionId]!,
            diagnostics: {
              provider: { kind: "retry", attempt: 2 },
              lastActivity: { kind: "message", receivedAt: 100 },
            },
          },
        },
      })),
    );
    await userEvent.setup().click(
      screen.getByRole("button", {
        name: appI18n.t("chat.sessionInformation"),
      }),
    );
    await screen.findByText(appI18n.t("chat.bindingNotFound"));
    expect(screen.getByTestId("run-diagnostics")).toHaveTextContent(
      appI18n.t("chat.diagnosticsRetry", { attempt: 2 }),
    );
    expect(screen.getByTestId("run-diagnostics")).toHaveTextContent(
      appI18n.t("chat.diagnosticsLastActivity"),
    );
    const group = document.querySelector('[data-subagent-group="child"]');
    await userEvent.setup().click(
      screen.getByRole("button", {
        name: `${appI18n.t("chat.bindingLocateChildren")} · Worker`,
      }),
    );
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(
      document.querySelectorAll('[data-subagent-group="child"]'),
    ).toHaveLength(1);
    expect(document.querySelector('[data-subagent-group="child"]')).toBe(group);
    await waitFor(() => expect(group?.querySelector("button")).toHaveFocus());
  });
  it("renders the node session through the ordinary chat surface", () => {
    renderDock("awaiting_input", false);

    expect(screen.getByRole("main")).toHaveClass("flex-1");
    // TipTap exposes the empty-state hint via data-placeholder, not a native input placeholder.
    expect(
      screen
        .getByRole("textbox")
        .querySelector("[data-placeholder]")
        ?.getAttribute("data-placeholder"),
    ).toMatch(/描述一个任务/);
  });

  it("keeps the node session's authoritative model visible", async () => {
    const nodeSessionOptions: acp.SessionConfigOption[] = [
      {
        id: "model",
        name: "Model",
        category: "model",
        type: "select",
        currentValue: "workflow/node-model",
        options: [{ value: "workflow/node-model", name: "Workflow Model" }],
      },
    ];
    renderDock(
      "awaiting_input",
      false,
      [],
      undefined,
      undefined,
      nodeSessionOptions,
    );

    const modelPicker = screen.getByRole("button", {
      name: appI18n.t("chat.modelSelector.label"),
    });
    expect(modelPicker).toBeDisabled();
    expect(modelPicker).toHaveTextContent("Workflow Model");

    expect(modelPicker).toHaveTextContent("Workflow Model");

    await userEvent.click(modelPicker);
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("places the shared conversation navigator inside the node session", () => {
    renderDock("awaiting_input", false, [
      {
        id: "turn-1",
        userMessage: {
          kind: "message",
          id: "turn-1-user",
          role: "user",
          content: "Inspect the workflow",
          createdAt: 1,
        },
        items: [
          {
            kind: "message",
            id: "turn-1-agent",
            role: "assistant",
            content: "Inspection complete",
            createdAt: 2,
          },
        ],
        status: "completed",
        stopReason: "end_turn",
        error: null,
        createdAt: 1,
      },
    ]);

    const anchorList = screen.getByTestId("conversation-anchor-list");
    const navigator = anchorList.closest("nav");
    expect(navigator).not.toBeNull();
    expect(navigator).toHaveClass("absolute", "right-0");
    expect(navigator).not.toHaveClass("fixed");
    expect(
      anchorList.querySelectorAll("[data-conversation-tick]"),
    ).toHaveLength(2);
  });

  it("loads a non-interactive node through the same surface without interaction controls", async () => {
    const loadSpy = renderReadOnlyDock();

    await waitFor(() =>
      expect(loadSpy).toHaveBeenCalledWith({ sessionId }, expect.anything()),
    );
    expect(screen.getByRole("main")).toBeInTheDocument();
    expect(screen.queryByRole("textbox")).toBeNull();
    expect(screen.queryByTestId("complete-current-node")).toBeNull();
    expect(
      screen.getByRole("button", { name: "返回阶段摘要" }),
    ).toBeInTheDocument();
  });

  it("does not retry a failed node session load after a runtime fault", async () => {
    const clientHandlers: TestHandlers =
      createFixtureHandlers(createFixtureState());
    const loadSpy = vi.fn(async function* () {
      yield* [];
      throw new Error("agent session unavailable");
    });
    clientHandlers.loadSession = loadSpy;
    const client = createTestClient(clientHandlers);
    const chatStore = createChatStore(client.session);

    render(
      <RunNodeSessionChat
        sessionId={sessionId}
        status="running"
        sessionActions={<button type="button">返回阶段摘要</button>}
      />,
      {
        wrapper: createHookWrapper(client, createTestQueryClient(), chatStore),
      },
    );

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "agent session unavailable",
    );
    expect(
      screen.queryByRole("status", { name: "正在加载历史记录…" }),
    ).toBeNull();
    expect(loadSpy).toHaveBeenCalledTimes(1);

    await act(async () => {
      await new Promise((resolve) => {
        window.setTimeout(resolve, 500);
      });
    });
    expect(loadSpy).toHaveBeenCalledTimes(1);
  });

  it("keeps replaying an empty running session until its automatic prompt appears", async () => {
    const clientHandlers: TestHandlers =
      createFixtureHandlers(createFixtureState());
    const client = createTestClient(clientHandlers);
    const loadSpy = vi.spyOn(client.session, "load");
    const chatStore = createChatStore(client.session);
    chatStore.setState({
      conversations: { [sessionId]: seededConversation(false) },
    });

    render(<RunNodeSessionChat sessionId={sessionId} status="running" />, {
      wrapper: createHookWrapper(client, createTestQueryClient(), chatStore),
    });

    expect(
      screen.getByRole("status", { name: "正在加载历史记录…" }),
    ).toBeInTheDocument();

    await waitFor(
      () => {
        expect(loadSpy.mock.calls.length).toBeGreaterThanOrEqual(2);
        expect(loadSpy).toHaveBeenLastCalledWith(
          { sessionId },
          expect.anything(),
        );
      },
      { timeout: 1_500 },
    );
    expect(
      screen.getByRole("status", { name: "正在加载历史记录…" }),
    ).toBeInTheDocument();
    expect(screen.queryByText("尚无消息")).toBeNull();
  });

  it("reveals a running node session as soon as its first turn is available", () => {
    const clientHandlers: TestHandlers =
      createFixtureHandlers(createFixtureState());
    const client = createTestClient(clientHandlers);
    const chatStore = createChatStore(client.session);
    chatStore.setState({
      conversations: { [sessionId]: seededConversation(false) },
    });
    render(<RunNodeSessionChat sessionId={sessionId} status="running" />, {
      wrapper: createHookWrapper(client, createTestQueryClient(), chatStore),
    });

    expect(
      screen.getByRole("status", { name: "正在加载历史记录…" }),
    ).toBeInTheDocument();

    act(() => {
      chatStore.setState({
        conversations: {
          [sessionId]: seededConversation(true, [
            {
              id: "turn-1",
              userMessage: {
                kind: "message",
                id: "turn-1-user",
                role: "user",
                content: "Automated workflow prompt",
                createdAt: 1,
              },
              items: [],
              status: "streaming",
              stopReason: null,
              error: null,
              createdAt: 1,
            },
          ]),
        },
      });
    });

    expect(
      screen.queryByRole("status", { name: "正在加载历史记录…" }),
    ).toBeNull();
    expect(screen.getByText("Automated workflow prompt")).toBeInTheDocument();
  });

  it("cancels the session's active prompt instead of aborting its load stream", async () => {
    const { client } = renderDock("running", true, [
      {
        id: "turn-1",
        userMessage: {
          kind: "message",
          id: "turn-1-user",
          role: "user",
          content: "Automated workflow prompt",
          createdAt: 1,
        },
        items: [
          {
            kind: "message",
            id: "turn-1-agent",
            role: "assistant",
            content: "Working",
            createdAt: 2,
          },
        ],
        status: "streaming",
        stopReason: null,
        error: null,
        createdAt: 1,
      },
    ]);
    const cancelSpy = vi.spyOn(client.session, "cancelPrompt");

    await userEvent.click(screen.getByRole("button", { name: /停止|stop/i }));

    await waitFor(() => expect(cancelSpy).toHaveBeenCalledWith({ sessionId }));
  });

  it("keeps the card fixed while the ordinary message list owns scrolling", () => {
    renderDock("awaiting_input", false, [
      {
        id: "turn-1",
        userMessage: {
          kind: "message",
          id: "turn-1-user",
          role: "user",
          content: "Inspect the workflow",
          createdAt: 1,
        },
        items: [],
        status: "completed",
        stopReason: null,
        error: null,
        createdAt: 1,
      },
    ]);

    expect(screen.getByTestId("message-list")).toHaveClass(
      "h-full",
      "overflow-y-auto",
    );
  });

  it("right-aligns completion and return actions without shifting the composer", () => {
    renderDock(
      "awaiting_input",
      false,
      [],
      <button type="button">返回阶段摘要</button>,
    );

    const actions = document.querySelector('[data-slot="composer-actions"]');
    const composerContainer = actions?.parentElement?.firstElementChild;
    expect(actions).not.toBeNull();
    expect(composerContainer).not.toBeNull();
    expect(actions).toHaveClass("absolute", "right-3");
    expect(actions?.parentElement).toHaveClass("relative");
    expect(composerContainer).toHaveClass(
      "mx-auto",
      "max-w-[760px]",
      "w-[calc(100%_-_13rem)]",
    );
    expect(
      within(actions as HTMLElement)
        .getAllByRole("button")
        .map(
          (button) => button.getAttribute("aria-label") ?? button.textContent,
        ),
    ).toEqual(["完成当前节点", "返回阶段摘要"]);
  });

  it("completes the node when the button is clicked while awaiting input", async () => {
    const { client } = renderDock(
      "awaiting_input",
      false,
      [],
      undefined,
      undefined,
      undefined,
      readyCompletion(),
    );
    const completeSpy = vi.spyOn(client.workflowRun, "completeNode");

    const button = screen.getByTestId("complete-current-node");
    expect(button).toBeEnabled();
    await userEvent.click(button);

    await waitFor(() =>
      expect(completeSpy).toHaveBeenCalledWith({
        runId,
        nodeId,
        force: false,
      }),
    );
  });

  it("shows completion progress until the request and run refresh settle", async () => {
    let releaseCompletion: () => void = () => {};
    const completionGate = new Promise<void>((resolve) => {
      releaseCompletion = resolve;
    });
    const onNodeCompleted = vi.fn();
    const { handlers } = renderDock(
      "awaiting_input",
      false,
      [],
      undefined,
      onNodeCompleted,
      undefined,
      readyCompletion(),
    );
    vi.spyOn(handlers, "completeWorkflowNode").mockImplementation(async () => {
      await completionGate;
      return {
        run: {
          id: runId,
          workspaceId: "workspace-1",
          workflowId: "workflow-1",
          snapshotId: "snapshot-1",
          name: "Run 1",
          status: "running",
          state: null,
          input: null,
          output: null,
          error: null,
          payload: null,
          startedAt: 1n,
          finishedAt: null,
          createdAt: 1n,
          updatedAt: 2n,
        },
      };
    });

    const button = screen.getByTestId("complete-current-node");
    await userEvent.click(button);

    await waitFor(() => expect(button).toHaveAttribute("aria-busy", "true"));
    expect(button.querySelector('[data-slot="spinner"]')).not.toBeNull();
    expect(onNodeCompleted).not.toHaveBeenCalled();

    releaseCompletion();

    await waitFor(() => expect(button).toHaveAttribute("aria-busy", "false"));
    expect(onNodeCompleted).toHaveBeenCalledWith(nodeId);
  });

  it("shows a ready step card that completes without force and names the next step", async () => {
    const { client } = renderDock(
      "awaiting_input",
      false,
      [],
      undefined,
      undefined,
      undefined,
      readyCompletion(),
      ["Review changes"],
    );
    const completeSpy = vi.spyOn(client.workflowRun, "completeNode");

    const card = screen.getByTestId("node-step-card");
    expect(within(card).getByText("等待参与")).toBeInTheDocument();
    const action = screen.getByTestId("complete-ready-node");
    expect(action).toHaveTextContent("完成并进入下一步：Review changes");
    expect(screen.getByTestId("continue-chat")).toHaveTextContent("继续对话");

    await userEvent.click(action);

    await waitFor(() =>
      expect(completeSpy).toHaveBeenCalledWith({
        runId,
        nodeId,
        force: false,
      }),
    );
  });

  it("omits the next step on the last node", () => {
    renderDock(
      "awaiting_input",
      false,
      [],
      undefined,
      undefined,
      undefined,
      readyCompletion(),
    );

    const action = screen.getByTestId("complete-ready-node");
    expect(action).toHaveTextContent("完成当前步骤");
    expect(action).not.toHaveTextContent("下一步");
  });

  it("keeps the ordinary completion entry for a node parked without a snapshot", async () => {
    const { client } = renderDock("awaiting_input", false, []);
    const completeSpy = vi.spyOn(client.workflowRun, "completeNode");

    const card = screen.getByTestId("node-step-card");
    expect(card).toHaveTextContent("本步骤还没有完成快照");
    expect(within(card).getByText("已暂停")).toBeInTheDocument();

    await userEvent.click(screen.getByTestId("complete-legacy-node"));

    await waitFor(() =>
      expect(completeSpy).toHaveBeenCalledWith({
        runId,
        nodeId,
        force: false,
      }),
    );
    expect(screen.getByTestId("force-complete-node")).toBeInTheDocument();
  });

  it("reports a rejected completion instead of failing silently", async () => {
    const errorToast = vi.spyOn(toast, "error").mockImplementation(() => "id");
    const { handlers } = renderDock(
      "awaiting_input",
      false,
      [],
      undefined,
      undefined,
      undefined,
      readyCompletion(),
    );
    handlers.completeWorkflowNode = async () => {
      throw new RemoteContractError(
        {
          code: "workflow_node_turn_changed",
          params: {
            nodeId,
            expected: { userSeq: 1, endedSeq: 2 },
            actual: { userSeq: 3, endedSeq: 4 },
          },
          requestId: "550e8400-e29b-41d4-a716-446655440000",
        },
        null,
      );
    };

    await userEvent.click(screen.getByTestId("complete-ready-node"));

    await waitFor(() =>
      expect(errorToast).toHaveBeenCalledWith(
        "本轮对话已变化，请刷新运行后重新确认。",
        { description: undefined, action: undefined },
      ),
    );
    errorToast.mockRestore();
  });

  it.each([
    ["turn_stopped", "已停止：本轮生成被中断。"],
    ["turn_truncated", "被截断：本轮达到输出上限。"],
    ["turn_refused", "拒答：智能体没有完成本轮请求。"],
    ["turn_unknown", "回合未知：没有找到本轮正常结束的记录。"],
    ["structured_invalid", "结构化输出不符合：本轮输出未通过声明的结构校验。"],
    ["output_missing", "没有输出：本轮没有产生声明的输出。"],
  ] as const)(
    "renders the %s blocker as a paused step card",
    (kind, reason) => {
      renderDock(
        "awaiting_input",
        false,
        [],
        undefined,
        undefined,
        undefined,
        blockedCompletion(kind),
      );

      const card = screen.getByTestId("node-step-card");
      expect(within(card).getByText("已暂停")).toBeInTheDocument();
      expect(card).toHaveTextContent("本轮未完成");
      expect(card).toHaveTextContent(reason);
      expect(screen.getByTestId("continue-current-step")).toBeInTheDocument();
      expect(screen.getByTestId("force-complete-node")).toBeInTheDocument();
    },
  );

  it("offers the ordinary completion for a provisional turn and re-checks it", async () => {
    const { client } = renderDock(
      "awaiting_input",
      false,
      [],
      undefined,
      undefined,
      undefined,
      blockedCompletion("turn_unknown", true),
    );
    const completeSpy = vi.spyOn(client.workflowRun, "completeNode");

    const card = screen.getByTestId("node-step-card");
    expect(card).toHaveTextContent("等待参与");
    expect(card).toHaveTextContent(
      "本轮仍在收尾；点完成时会按本轮重新校验，未结束会给出原因。",
    );
    expect(within(card).getByTestId("force-complete-node")).toBeInTheDocument();
    expect(screen.getByTestId("complete-current-node")).toBeEnabled();

    await userEvent.click(screen.getByTestId("complete-ready-node"));

    await waitFor(() =>
      expect(completeSpy).toHaveBeenCalledWith({
        runId,
        nodeId,
        force: false,
      }),
    );
  });

  it("keeps the opening step awaiting confirmation instead of offering to advance", () => {
    renderDock(
      "awaiting_input",
      false,
      [],
      undefined,
      undefined,
      undefined,
      blockedCompletion("turn_unconfirmed"),
      ["Review changes"],
    );

    const card = screen.getByTestId("node-step-card");
    expect(within(card).getByText("等待参与")).toBeInTheDocument();
    expect(card).toHaveTextContent("等待你的确认");
    expect(card).toHaveTextContent(
      "本次停靠来自 workflow 的首轮自动回合，不代表本步骤已完成。",
    );
    expect(card).not.toHaveTextContent("完成并进入下一步");
    expect(screen.queryByTestId("complete-ready-node")).toBeNull();
    expect(screen.getByTestId("continue-current-step")).toBeInTheDocument();
    expect(screen.getByTestId("force-complete-node")).toBeInTheDocument();
    expect(screen.getByTestId("complete-current-node")).toBeDisabled();
  });

  it("sends the unconfirmed scoped turn when continuing the opening step", async () => {
    const { client } = renderDock(
      "awaiting_input",
      false,
      [],
      undefined,
      undefined,
      undefined,
      blockedCompletion("turn_unconfirmed"),
    );
    const promptSpy = vi.spyOn(client.session, "prompt");

    await userEvent.click(screen.getByTestId("continue-current-step"));

    await waitFor(() => expect(promptSpy).toHaveBeenCalledTimes(1));
    const request = promptSpy.mock.calls[0]![0];
    const text = request.prompt
      .flatMap((block) => (block.type === "text" ? [block.text] : []))
      .join("\n");
    expect(text).toContain("本轮自动回合已经结束");
    expect(text).toContain("不要执行后续节点");
    expect(text).not.toContain("本轮没有正常结束");
  });

  it("confirms the opening step before force-completing it", async () => {
    const { client } = renderDock(
      "awaiting_input",
      false,
      [],
      undefined,
      undefined,
      undefined,
      blockedCompletion("turn_unconfirmed"),
    );
    const completeSpy = vi.spyOn(client.workflowRun, "completeNode");

    await userEvent.click(screen.getByTestId("force-complete-node"));

    const dialog = await screen.findByRole("alertdialog");
    expect(dialog).toHaveTextContent("本轮已正常结束，但还没有你的确认。");
    await userEvent.click(within(dialog).getByTestId("confirm-force-complete"));

    await waitFor(() =>
      expect(completeSpy).toHaveBeenCalledWith({
        runId,
        nodeId,
        force: true,
      }),
    );
  });

  it("shows non-blocking warnings from a ready turn", () => {
    renderDock("awaiting_input", false, [], undefined, undefined, undefined, {
      ...readyCompletion(),
      warnings: ["empty_output"],
    });

    const card = screen.getByTestId("node-step-card");
    expect(card).toHaveTextContent(
      "本轮没有文本输出；该节点未声明输出契约，仍可完成。",
    );
    expect(card).toHaveTextContent("等待参与");
  });

  it("sends a scoped human turn when continuing an incomplete step", async () => {
    const { client } = renderDock(
      "awaiting_input",
      false,
      [],
      undefined,
      undefined,
      undefined,
      blockedCompletion("turn_stopped"),
    );
    const promptSpy = vi.spyOn(client.session, "prompt");

    await userEvent.click(screen.getByTestId("continue-current-step"));

    await waitFor(() => expect(promptSpy).toHaveBeenCalledTimes(1));
    const request = promptSpy.mock.calls[0]![0];
    expect(request.sessionId).toBe(sessionId);
    const text = request.prompt
      .flatMap((block) => (block.type === "text" ? [block.text] : []))
      .join("\n");
    expect(text).toContain("请继续当前步骤");
    expect(text).toContain("已停止");
    expect(text).toContain("不要执行后续节点");
  });

  it("sends a human turn when continuing a ready step", async () => {
    const { client } = renderDock(
      "awaiting_input",
      false,
      [],
      undefined,
      undefined,
      undefined,
      readyCompletion(),
    );
    const promptSpy = vi.spyOn(client.session, "prompt");

    await userEvent.click(screen.getByTestId("continue-chat"));

    await waitFor(() => expect(promptSpy).toHaveBeenCalledTimes(1));
    const request = promptSpy.mock.calls[0]![0];
    const text = request.prompt
      .flatMap((block) => (block.type === "text" ? [block.text] : []))
      .join("\n");
    expect(text).toContain("请继续对话");
    expect(text).toContain("暂时不要结束当前步骤");
  });

  it("requires confirmation before force-completing an incomplete step", async () => {
    const { client } = renderDock(
      "awaiting_input",
      false,
      [],
      undefined,
      undefined,
      undefined,
      blockedCompletion("turn_refused"),
    );
    const completeSpy = vi.spyOn(client.workflowRun, "completeNode");

    await userEvent.click(screen.getByTestId("force-complete-node"));
    expect(completeSpy).not.toHaveBeenCalled();

    const dialog = await screen.findByRole("alertdialog");
    expect(dialog).toHaveTextContent("仍然完成当前步骤？");
    await userEvent.click(within(dialog).getByTestId("confirm-force-complete"));

    await waitFor(() =>
      expect(completeSpy).toHaveBeenCalledWith({
        runId,
        nodeId,
        force: true,
      }),
    );
  });

  it("disables the complete button while the agent is responding", () => {
    renderDock("awaiting_input", true);
    expect(screen.getByTestId("complete-current-node")).toBeDisabled();
  });

  it("disables the bottom-right completion action while blockers remain", () => {
    renderDock(
      "awaiting_input",
      false,
      [],
      undefined,
      undefined,
      undefined,
      blockedCompletion("turn_truncated"),
    );

    expect(screen.getByTestId("complete-current-node")).toBeDisabled();
  });

  it("hides the complete button once the node is terminal", () => {
    renderDock(
      "succeeded",
      false,
      [],
      <button type="button">返回阶段摘要</button>,
    );
    expect(screen.queryByTestId("complete-current-node")).toBeNull();
    expect(
      screen.getByRole("button", { name: "返回阶段摘要" }),
    ).toBeInTheDocument();
  });
});
