import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import {
  createChatStore,
  type ChatTurn,
  type SessionConversation,
} from "@ora/chat";
import type {
  WorkflowNodeConversationItem,
  WorkflowNodeData,
} from "@ora/workflow-runtime";
import {
  createHookWrapper,
  createTestQueryClient,
} from "../../test/hook-harness";
import {
  createTestClient,
  type TestHandlers,
} from "../../test/contracts-transport";
import { createPluginMemory, pluginHandlers } from "../../test/memory/plugins";
import { createAgentMemory, agentHandlers } from "../../test/memory/agents";
import "../../i18n/i18n-instance";
import { RunTheaterActCard } from "./run-theater-act-card";
import { AGENT_REF } from "../../test/agent-identity";

/** State for this test surface; no unrelated domain fixtures are initialized. */
function createFixtureState() {
  return { ...createPluginMemory(), ...createAgentMemory() };
}

type FixtureState = ReturnType<typeof createFixtureState>;

/** Explicit domain composition for the behaviors exercised by this test file. */
function createFixtureHandlers(state: FixtureState): TestHandlers {
  return {
    ...pluginHandlers(state),
    ...agentHandlers(state),
  };
}

const NODE_DATA: WorkflowNodeData = {
  kind: "agent",
  title: "Review changes",
  description: "Review the current branch",
  instruction: "Find regressions and summarize them.",
  model: "mock-model",
};

it("renders condition semantics without Agent instruction, output or conversation placeholders", () => {
  render(
    <RunTheaterActCard
      data={{
        kind: "condition",
        title: "Review decision",
        description: "",
        conditionCases: [
          {
            id: "pass",
            conditions: [
              {
                variableSelector: ["reviewer", "output"],
                operator: "eq",
                value: "PASS",
              },
            ],
          },
        ],
      }}
      state={{ status: "succeeded" }}
      live={false}
      showCompletedOutput
    />,
    { wrapper: createSessionWrapper() },
  );
  expect(screen.getByText("分支条件")).toBeInTheDocument();
  expect(screen.getByText(/reviewer.output/)).toBeInTheDocument();
  expect(screen.getByText("判断结果")).toBeInTheDocument();
  expect(screen.getByText(/当前运行记录未提供命中的分支/)).toBeInTheDocument();
  expect(screen.queryByText("指令")).not.toBeInTheDocument();
  expect(screen.queryByText("输出")).not.toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: /查看.*对话/ }),
  ).not.toBeInTheDocument();
});

it("highlights the selected branch and renders only that case's rules across round changes", () => {
  const data: WorkflowNodeData = {
    kind: "condition",
    title: "Decision",
    description: "",
    cases: [
      {
        id: "pass",
        conditions: [
          {
            variableSelector: ["reviewer", "output"],
            operator: "eq",
            value: "PASS",
          },
        ],
      },
      {
        id: "stop",
        conditions: [
          {
            variableSelector: ["reviewer", "output"],
            operator: "eq",
            value: "STOP",
          },
        ],
      },
    ],
  };
  const view = render(
    <RunTheaterActCard
      data={data}
      state={{ status: "succeeded", selectedBranchId: "else" }}
      live={false}
    />,
    { wrapper: createSessionWrapper() },
  );
  expect(screen.getByText("本轮选择：ELSE")).toBeInTheDocument();
  expect(screen.getAllByText(/reviewer.output.*PASS/)).toHaveLength(1);
  expect(screen.getAllByText(/reviewer.output.*STOP/)).toHaveLength(1);
  expect(screen.queryByText(/PASS.*STOP/)).not.toBeInTheDocument();
  view.rerender(
    <RunTheaterActCard
      data={data}
      state={{ status: "succeeded", selectedBranchId: "pass" }}
      live={false}
    />,
  );
  expect(screen.getByText("本轮选择：pass")).toBeInTheDocument();
  expect(screen.queryByText("本轮选择：ELSE")).not.toBeInTheDocument();
  expect(screen.getAllByText("本轮命中")).toHaveLength(1);
});

const CONVERSATION: WorkflowNodeConversationItem[] = [
  {
    kind: "message",
    id: "message-user-1",
    runId: "run-1",
    nodeId: "review",
    sessionId: "session-1",
    role: "user",
    markdown: "Review src/auth.ts and report regressions.",
    status: "complete",
    createdAt: "2026-08-04T12:00:00+08:00",
    updatedAt: "2026-08-04T12:00:00+08:00",
  },
];

const CHAT_TURNS: ChatTurn[] = [
  {
    id: "turn-1",
    userMessage: {
      kind: "message",
      id: "message-user-1",
      role: "user",
      content: "Review src/auth.ts and report regressions.",
      createdAt: 1,
    },
    items: [
      {
        kind: "message",
        id: "message-assistant-1",
        role: "assistant",
        content: "## Review complete\n\n- Found **one** regression.",
        createdAt: 2,
      },
    ],
    status: "completed",
    stopReason: "end_turn",
    error: null,
    createdAt: 1,
  },
];

/** Supplies the same loaded store shape used by an ordinary session page. */
function loadedConversation(): SessionConversation {
  return {
    configOptions: [],
    modelChanges: [],
    historyNotices: [],
    turns: CHAT_TURNS,
    availableCommands: [],
    sessionTitle: null,
    sessionUpdatedAt: null,
    isLoaded: true,
    isLoading: false,
    isResponding: false,
    pendingPermissions: [],
    usage: {
      context: { status: "hidden" },
      lastTurnTokens: { status: "none" },
    },
    error: null,
  };
}

/** Builds application providers around a loaded ordinary session. */
function createSessionWrapper() {
  const clientHandlers: TestHandlers =
    createFixtureHandlers(createFixtureState());
  const client = createTestClient(clientHandlers);
  const chatStore = createChatStore(client.session);
  chatStore.setState({
    conversations: { "session-1": loadedConversation() },
  });
  return createHookWrapper(client, createTestQueryClient(), chatStore);
}

/** Renders a card with the application providers and a loaded ordinary session. */
function renderSessionCard(onToggleInspector = vi.fn()) {
  const wrapper = createSessionWrapper();
  const renderView = (inspectorOpen: boolean) => (
    <RunTheaterActCard
      data={NODE_DATA}
      state={{ status: "succeeded", sessionId: "session-1" }}
      live={false}
      conversation={CONVERSATION}
      inspectorOpen={inspectorOpen}
      onToggleInspector={onToggleInspector}
    />
  );
  const view = render(renderView(false), { wrapper });
  return {
    user: userEvent.setup(),
    setInspectorOpen: (open: boolean) => view.rerender(renderView(open)),
  };
}

describe("RunTheaterActCard conversation", () => {
  it("renders Output results without exposing an Agent session dock", () => {
    render(
      <RunTheaterActCard
        data={{
          kind: "output",
          title: "Output",
          description: "Return changed files",
          outputs: [
            {
              name: "files",
              variableSelector: ["agent-5", "structured_output", "files"],
            },
          ],
        }}
        state={{
          status: "succeeded",
          output: {
            summary: JSON.stringify({
              files: [
                {
                  file_path: "src/vs/base/common/numbers.ts",
                  lines: [
                    {
                      symbol: "formatTokenCount",
                      start_line: 15,
                      end_line: 26,
                    },
                  ],
                },
              ],
            }),
          },
        }}
        live={false}
        conversationOpen
        onConversationOpenChange={vi.fn()}
      />,
      { wrapper: createSessionWrapper() },
    );

    expect(
      screen.queryByText(
        /Agent (正在处理|is working|尚未启动|has not started)/,
      ),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /查看节点会话|View node session/ }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByText(/src\/vs\/base\/common\/numbers\.ts/),
    ).toHaveTextContent('"start_line": 15');
  });

  it("shows not-started copy when the session is open before the run starts", () => {
    render(
      <RunTheaterActCard
        data={NODE_DATA}
        state={{ status: "idle" }}
        live={false}
        conversationOpen
        onConversationOpenChange={vi.fn()}
      />,
      { wrapper: createSessionWrapper() },
    );

    expect(screen.getByText("Agent 尚未启动")).toBeInTheDocument();
    expect(screen.queryByText("Agent 正在处理")).not.toBeInTheDocument();
  });

  it("keeps working copy while the node is running without a session yet", () => {
    render(
      <RunTheaterActCard
        data={NODE_DATA}
        state={{ status: "running" }}
        live
        conversationOpen
        onConversationOpenChange={vi.fn()}
      />,
      { wrapper: createSessionWrapper() },
    );

    expect(screen.getByText("Agent 正在处理")).toBeInTheDocument();
  });

  it("marks automatic and interactive Agent nodes beside the title", () => {
    const automaticAgent = {
      schemaVersion: 3 as const,
      executor: { agentCli: "open_code", modelId: "mock-model" },
      roleId: "reviewer",
      skills: [],
      mcps: [],
      prompt: "Review the current branch.",
      interactive: false,
    };
    const wrapper = createSessionWrapper();
    const renderCard = (interactive: boolean) => (
      <RunTheaterActCard
        data={{
          ...NODE_DATA,
          agentConfig: { ...automaticAgent, interactive },
        }}
        state={{ status: "succeeded" }}
        live={false}
      />
    );
    const view = render(renderCard(false), { wrapper });

    const automaticMark = screen.getByRole("img", {
      name: "自动执行节点",
    });
    expect(automaticMark).toHaveAttribute(
      "data-agent-execution-mode",
      "automatic",
    );
    expect(
      screen.getByRole("heading", { name: "Review changes" })
        .nextElementSibling,
    ).toBe(automaticMark);

    view.rerender(renderCard(true));

    const interactiveMark = screen.getByRole("img", {
      name: "人工交互节点",
    });
    expect(interactiveMark).toHaveAttribute(
      "data-agent-execution-mode",
      "interactive",
    );
  });

  it("shows agent prompt and executor detail when flat fields are absent", async () => {
    const user = userEvent.setup();
    const longPrompt = "梳理现状、约束、风险与可选路径。".repeat(8);
    render(
      <RunTheaterActCard
        data={{
          kind: "agent",
          title: "探索",
          description: "只读探索",
          agentConfig: {
            schemaVersion: 3,
            executor: {
              agentCli: AGENT_REF.opencode,
              modelId: "deepseek/deepseek-v4-flash",
            },
            roleId: "researcher",
            skills: [],
            mcps: [],
            prompt: longPrompt,
          },
        }}
        state={{ status: "succeeded" }}
        live={false}
      />,
      { wrapper: createSessionWrapper() },
    );

    expect(
      await screen.findByText("OpenCode · deepseek/deepseek-v4-flash"),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "查看完整指令" }));
    expect(await screen.findByRole("dialog")).toHaveTextContent(longPrompt);
  });

  it("renders a non-interactive node through the ordinary session surface", async () => {
    const { user } = renderSessionCard();

    await user.click(screen.getByRole("button", { name: "查看节点会话" }));

    expect(
      document.querySelector('[data-workflow-node-chrome="stage"]'),
    ).toHaveClass("h-full", "max-w-none", "overflow-hidden");
    expect(
      screen.getByText("Review src/auth.ts and report regressions."),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: "Review complete" }),
    ).toBeInTheDocument();
    expect(screen.getByTestId("message-list")).toHaveClass("overflow-y-auto");
    expect(screen.queryByRole("textbox")).toBeNull();
    expect(screen.queryByTestId("complete-current-node")).toBeNull();
    expect(
      screen.getByRole("button", { name: "返回阶段摘要" }),
    ).toBeInTheDocument();
  });

  it("opens the inspector only from its header button in session mode", async () => {
    const onToggleInspector = vi.fn();
    const { setInspectorOpen, user } = renderSessionCard(onToggleInspector);

    await user.click(screen.getByText("Review the current branch"));
    expect(onToggleInspector).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "打开节点详情" }));
    expect(onToggleInspector).toHaveBeenCalledTimes(1);

    setInspectorOpen(true);
    const closeButton = screen.getByRole("button", { name: "关闭阶段详情" });
    expect(closeButton).toHaveAttribute("aria-pressed", "true");

    await user.click(screen.getByRole("button", { name: "查看节点会话" }));
    await user.click(closeButton);
    expect(onToggleInspector).toHaveBeenCalledTimes(2);

    await user.click(screen.getByRole("button", { name: "返回阶段摘要" }));
    expect(onToggleInspector).toHaveBeenCalledTimes(2);
  });

  it("labels a parked interactive turn as paused without printing the raw stop reason", () => {
    render(
      <RunTheaterActCard
        data={{
          ...NODE_DATA,
          agentConfig: {
            schemaVersion: 3,
            executor: { agentCli: AGENT_REF.opencode, modelId: "mock-model" },
            roleId: "reviewer",
            skills: [],
            mcps: [],
            prompt: "Review the current branch.",
            interactive: true,
          },
        }}
        state={{
          status: "awaiting_input",
          stopReason: "cancelled",
          completion: {
            blockers: [{ kind: "turn_stopped", params: {} }],
            warnings: [],
            provisional: false,
            ready: false,
          },
        }}
        live
      />,
      { wrapper: createSessionWrapper() },
    );

    expect(screen.getByText("已暂停")).toBeInTheDocument();
    expect(screen.queryByText("cancelled")).toBeNull();
  });

  it("keeps a provisional turn out of the paused badge", () => {
    render(
      <RunTheaterActCard
        data={{
          ...NODE_DATA,
          agentConfig: {
            schemaVersion: 3,
            executor: { agentCli: AGENT_REF.opencode, modelId: "mock-model" },
            roleId: "reviewer",
            skills: [],
            mcps: [],
            prompt: "Review the current branch.",
            interactive: true,
          },
        }}
        state={{
          status: "awaiting_input",
          completion: {
            blockers: [{ kind: "turn_unknown", params: {} }],
            warnings: ["turn_not_settled"],
            provisional: true,
            ready: false,
          },
        }}
        live
      />,
      { wrapper: createSessionWrapper() },
    );

    expect(screen.getByText("等待参与")).toBeInTheDocument();
    expect(screen.queryByText("已暂停")).toBeNull();
  });

  it("keeps an unconfirmed opening turn out of the paused badge", () => {
    render(
      <RunTheaterActCard
        data={{
          ...NODE_DATA,
          agentConfig: {
            schemaVersion: 3,
            executor: { agentCli: AGENT_REF.opencode, modelId: "mock-model" },
            roleId: "reviewer",
            skills: [],
            mcps: [],
            prompt: "Review the current branch.",
            interactive: true,
          },
        }}
        state={{
          status: "awaiting_input",
          completion: {
            blockers: [{ kind: "turn_unconfirmed", params: {} }],
            warnings: [],
            provisional: false,
            ready: false,
          },
        }}
        live
      />,
      { wrapper: createSessionWrapper() },
    );

    expect(screen.getByText("等待参与")).toBeInTheDocument();
    expect(screen.queryByText("已暂停")).toBeNull();
  });

  it("keeps the session dock available while HITL interaction owns the footer", async () => {
    const user = userEvent.setup();
    render(
      <RunTheaterActCard
        data={NODE_DATA}
        state={{ status: "awaiting_input", sessionId: "session-1" }}
        live
        conversation={CONVERSATION}
        interaction={({ accessory }) => (
          <div>
            <div>HITL composer stub</div>
            {accessory}
          </div>
        )}
      />,
      { wrapper: createSessionWrapper() },
    );

    expect(screen.getByText("HITL composer stub")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "查看节点会话" }));
    expect(
      screen.getByRole("heading", { name: "Review complete" }),
    ).toBeInTheDocument();
    expect(screen.getByText("HITL composer stub")).toBeInTheDocument();
  });
});
