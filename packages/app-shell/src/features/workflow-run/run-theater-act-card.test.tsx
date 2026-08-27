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
  createMockClient,
  createMockClientState,
} from "../../test/mock-client";
import { RunTheaterActCard } from "./run-theater-act-card";

const NODE_DATA: WorkflowNodeData = {
  kind: "agent",
  title: "Review changes",
  description: "Review the current branch",
  instruction: "Find regressions and summarize them.",
  model: "mock-model",
};

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
    error: null,
  };
}

/** Builds application providers around a loaded ordinary session. */
function createSessionWrapper() {
  const client = createMockClient(createMockClientState());
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
              agentCli: "ora-space.opencode",
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
