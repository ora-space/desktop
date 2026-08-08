import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { AppI18nProvider } from "../../i18n/i18n";
import { RunTheaterActCard } from "./run-theater-act-card";
import type {
  WorkflowNodeConversationItem,
  WorkflowNodeData,
} from "@ora/workflow-runtime";

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
  {
    kind: "activity",
    activityKind: "thought",
    id: "activity-1",
    runId: "run-1",
    nodeId: "review",
    sessionId: "session-1",
    summary: "分析节点上下文",
    detail: "Mock thought",
    status: "complete",
    createdAt: "2026-08-04T12:00:01+08:00",
    updatedAt: "2026-08-04T12:00:01+08:00",
  },
  {
    kind: "message",
    id: "message-assistant-1",
    runId: "run-1",
    nodeId: "review",
    sessionId: "session-1",
    role: "assistant",
    markdown: "## Review complete\n\n- Found **one** regression.",
  status: "complete",
  createdAt: "2026-08-04T12:00:00+08:00",
  updatedAt: "2026-08-04T12:00:00+08:00",
  },
  {
    kind: "message",
    id: "message-user-2",
    runId: "run-1",
    nodeId: "review",
    sessionId: "session-1",
    role: "user",
    markdown: "Please double-check the auth boundary.",
    status: "complete",
    createdAt: "2026-08-04T12:00:02+08:00",
    updatedAt: "2026-08-04T12:00:02+08:00",
  },
  {
    kind: "message",
    id: "message-assistant-2",
    runId: "run-1",
    nodeId: "review",
    sessionId: "session-1",
    role: "assistant",
    markdown: "### Follow-up\n\nThe auth boundary looks consistent.",
    status: "complete",
    createdAt: "2026-08-04T12:00:03+08:00",
    updatedAt: "2026-08-04T12:00:03+08:00",
  },
];

/** Renders the stage card under the same i18n provider as the workspace. */
function renderCard(onSelect = vi.fn()) {
  return {
    onSelect,
    user: userEvent.setup(),
    ...render(
      <AppI18nProvider>
        <RunTheaterActCard
          data={NODE_DATA}
          state={{
            status: "succeeded",
            sessionId: "session-1",
            input: {
              summary: "Review current changes",
              detail: "Review src/auth.ts and report regressions.",
            },
          }}
          live={false}
          conversation={CONVERSATION}
          onSelect={onSelect}
        />
      </AppI18nProvider>,
    ),
  };
}

describe("RunTheaterActCard conversation", () => {
  it("shows agent prompt and executor mono detail when flat fields are absent", async () => {
    const user = userEvent.setup();
    const longPrompt = "梳理现状、约束、风险与可选路径。".repeat(8);
    render(
      <AppI18nProvider>
        <RunTheaterActCard
          data={{
            kind: "agent",
            title: "探索",
            description: "只读探索",
            agentConfig: {
              schemaVersion: 3,
              executor: { agentCli: "open_code", modelId: "deepseek/deepseek-v4-flash" },
              roleId: "researcher",
              skills: [],
              mcps: [],
              prompt: longPrompt,
            },
          }}
          state={{ status: "succeeded" }}
          live={false}
        />
      </AppI18nProvider>,
    );

    expect(screen.getByText("OpenCode · deepseek/deepseek-v4-flash")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "查看完整指令" }));
    expect(await screen.findByRole("dialog")).toHaveTextContent(longPrompt);
  });

  it("morphs the card body into a chat-like filtered transcript", async () => {
    const { user } = renderCard();

    await user.click(screen.getByRole("button", { name: "查看节点会话" }));

    expect(screen.getByText("Review src/auth.ts and report regressions.")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Review complete" })).toBeInTheDocument();
    expect(screen.queryByText("Find regressions and summarize them.")).not.toBeInTheDocument();
    expect(screen.getByText("已隐藏 1 条过程消息")).toBeInTheDocument();
    expect(screen.getByTestId("node-conversation-scroll")).toHaveClass("overflow-y-auto");
    expect(screen.getByTestId("conversation-anchor-list")).toBeInTheDocument();
  });

  it("keeps process activity collapsed until the reader asks for it", async () => {
    const { user } = renderCard();

    await user.click(screen.getByRole("button", { name: "查看节点会话" }));
    expect(screen.queryByText("Mock thought")).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /已隐藏 1 条过程消息/ }));
    expect(screen.getByText("Mock thought")).toBeInTheDocument();
  });

  it("keeps session-dock clicks separate from the act inspector action", async () => {
    const onSelect = vi.fn();
    const { user } = renderCard(onSelect);

    await user.click(screen.getByRole("button", { name: "查看节点会话" }));
    expect(onSelect).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "返回阶段摘要" }));
    expect(onSelect).not.toHaveBeenCalled();
  });

  it("keeps the session dock available while HITL interaction owns the footer", async () => {
    const user = userEvent.setup();
    render(
      <AppI18nProvider>
        <RunTheaterActCard
          data={NODE_DATA}
          state={{
            status: "awaiting_input",
            sessionId: "session-1",
            input: {
              summary: "Clarify auth scope",
              detail: "Need a decision before continuing.",
            },
          }}
          live
          conversation={CONVERSATION}
          interaction={({ accessory }) => (
            <div>
              <div>HITL composer stub</div>
              {accessory}
            </div>
          )}
        />
      </AppI18nProvider>,
    );

    expect(screen.getByText("HITL composer stub")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "查看节点会话" }));
    expect(screen.getByRole("heading", { name: "Review complete" })).toBeInTheDocument();
    expect(screen.getByText("HITL composer stub")).toBeInTheDocument();
  });
});
