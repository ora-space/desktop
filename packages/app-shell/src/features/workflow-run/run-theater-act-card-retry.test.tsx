import { act, cleanup, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createChatStore } from "@ora/chat";
import type {
  GraphWorkflowNodeState,
  WorkflowNodeData,
  WorkflowNodeRetryWait,
} from "@ora/workflow-runtime";
import {
  createHookWrapper,
  createTestQueryClient,
} from "../../test/hook-harness";
import { createTestClient } from "../../test/contracts-transport";
import { createPluginMemory, pluginHandlers } from "../../test/memory/plugins";
import { createAgentMemory, agentHandlers } from "../../test/memory/agents";
import { appI18n } from "../../i18n/i18n-instance";
import { formatRunClock } from "../../lib/format";
import { RunTheaterActCard } from "./run-theater-act-card";

// Records every mount of the node session surface; a waiting retry has no session to show.
const sessionChatSpy = vi.hoisted(() => vi.fn());

vi.mock("./run-node-session-chat", async () => {
  const { createElement } = await import("react");
  return {
    RunNodeSessionChat: (props: { sessionId: string; status: string }) => {
      sessionChatSpy(props);
      return createElement("div", {
        "data-testid": "node-session-chat",
        "data-session-id": props.sessionId,
        "data-status": props.status,
      });
    },
  };
});

const NOW = Date.UTC(2026, 8, 24, 6, 0, 0);
const STARTED_AT = "2026-09-24T13:59:10+08:00";
const FINISHED_AT = "2026-09-24T13:59:40+08:00";
const SESSION_PENDING =
  "上一次尝试失败，Ora 会自动重新运行这个节点；新一次尝试开始后，这里会显示它的会话。";

const WAIT: WorkflowNodeRetryWait = {
  attempt: 2,
  maxAttempt: 4,
  retry: 1,
  maxRetries: 3,
  delayMs: 30_000,
  scheduledAt: NOW,
  dueAt: NOW + 30_000,
};

const WAITING_STATE: GraphWorkflowNodeState = {
  status: "retry_waiting",
  retryWait: WAIT,
  autoRetry: { retry: 1, maxRetries: 3 },
};

const NODE_DATA: WorkflowNodeData = {
  kind: "agent",
  title: "Review changes",
  description: "Review the current branch",
  instruction: "Find regressions and summarize them.",
  model: "mock-model",
};

/** Application providers for the card; the session surface itself is stubbed above. */
function createWrapper() {
  const state = { ...createPluginMemory(), ...createAgentMemory() };
  const client = createTestClient({
    ...pluginHandlers(state),
    ...agentHandlers(state),
  });
  return createHookWrapper(
    client,
    createTestQueryClient(),
    createChatStore(client.session),
  );
}

beforeEach(async () => {
  await appI18n.changeLanguage("zh-CN");
  sessionChatSpy.mockClear();
  vi.useFakeTimers();
  vi.setSystemTime(NOW);
});

afterEach(() => {
  // Unmount while the fake clock is still installed so the countdown timer is cleared on it.
  cleanup();
  vi.useRealTimers();
});

describe("RunTheaterActCard while waiting to retry", () => {
  it("shows the live waiting label in place of the time range, without spinner or breathe", () => {
    const renderCard = (live: boolean) => (
      <RunTheaterActCard
        data={NODE_DATA}
        // A waiting row carrying stale timing must still not show a time range.
        state={{
          ...WAITING_STATE,
          startedAt: STARTED_AT,
          finishedAt: FINISHED_AT,
        }}
        live={live}
      />
    );
    const view = render(renderCard(false), { wrapper: createWrapper() });

    const card = screen.getByRole("article", {
      name: "Review changes: 等待重试",
    });
    const label = within(card).getByText("等待重试（第 2/4 次），30 秒后开始");
    expect(label).toHaveAttribute("data-retry-wait", "full");
    expect(label).toHaveAttribute("aria-live", "off");
    expect(label.parentElement).toHaveClass("text-orange-700");

    const badge = within(card).getByText("等待重试");
    expect(badge).toHaveClass(
      "border-orange-500/30",
      "bg-orange-500/10",
      "text-orange-800",
    );
    expect(
      badge.querySelector('[data-status-mark="retry_waiting"]'),
    ).not.toBeNull();
    expect(card).toHaveClass("border-orange-500/45", "ring-orange-500/15");
    expect(card).not.toHaveClass("ring-sky-500/35");

    expect(card.querySelector(".tabler-icon-loader-2")).toBeNull();
    expect(card.querySelector(".tabler-icon-refresh")).not.toBeNull();
    expect(
      document.querySelector('[class*="theater-live-breathe"]'),
    ).toBeNull();
    expect(card).not.toHaveTextContent(formatRunClock(STARTED_AT, "zh-CN"));
    expect(card).not.toHaveTextContent(formatRunClock(FINISHED_AT, "zh-CN"));

    // Even a caller that flags the card live gets no working cue for a waiting retry.
    view.rerender(renderCard(true));
    expect(card.querySelector(".tabler-icon-loader-2")).toBeNull();
    expect(
      document.querySelector('[class*="theater-live-breathe"]'),
    ).toBeNull();
    expect(card).not.toHaveClass("ring-sky-500/35");
  });

  it("counts the waiting label down each second and says starting at the deadline", () => {
    render(
      <RunTheaterActCard data={NODE_DATA} state={WAITING_STATE} live={false} />,
      { wrapper: createWrapper() },
    );
    const card = screen.getByRole("article", {
      name: "Review changes: 等待重试",
    });
    expect(
      within(card).getByText("等待重试（第 2/4 次），30 秒后开始"),
    ).toBeInTheDocument();

    act(() => {
      vi.advanceTimersByTime(1_000);
    });
    expect(
      within(card).getByText("等待重试（第 2/4 次），29 秒后开始"),
    ).toBeInTheDocument();
    expect(
      within(card).queryByText("等待重试（第 2/4 次），30 秒后开始"),
    ).not.toBeInTheDocument();

    // Each tick re-arms the next timer after React commits, so advance one second at a time.
    for (let tick = 0; tick < 29; tick += 1) {
      act(() => {
        vi.advanceTimersByTime(1_000);
      });
    }
    expect(
      within(card).getByText("等待重试（第 2/4 次），即将开始"),
    ).toBeInTheDocument();
  });

  it("explains the pending session instead of mounting a session chat", () => {
    render(
      <RunTheaterActCard
        data={NODE_DATA}
        state={WAITING_STATE}
        live={false}
        conversationOpen
        onConversationOpenChange={vi.fn()}
      />,
      { wrapper: createWrapper() },
    );

    expect(screen.getByText(SESSION_PENDING)).toBeInTheDocument();
    expect(screen.queryByText("Agent 正在处理")).not.toBeInTheDocument();
    expect(screen.queryByText("Agent 尚未启动")).not.toBeInTheDocument();
    expect(screen.queryByTestId("node-session-chat")).not.toBeInTheDocument();
    expect(sessionChatSpy).not.toHaveBeenCalled();
    expect(
      screen.getByRole("article", { name: "Review changes: 等待重试" }),
    ).toBeInTheDocument();
  });

  it("keeps a running node with a session on its time range, working cue and session chat", () => {
    const runningState: GraphWorkflowNodeState = {
      status: "running",
      sessionId: "session-1",
      startedAt: STARTED_AT,
    };
    const renderCard = (conversationOpen: boolean) => (
      <RunTheaterActCard
        data={NODE_DATA}
        state={runningState}
        live
        conversationOpen={conversationOpen}
        onConversationOpenChange={vi.fn()}
      />
    );
    const view = render(renderCard(false), { wrapper: createWrapper() });

    const card = screen.getByRole("article", {
      name: "Review changes: 运行中",
    });
    expect(
      within(card).getByText(`${formatRunClock(STARTED_AT, "zh-CN")} — —`),
    ).toBeInTheDocument();
    expect(card).toHaveClass("theater-live-breathe", "ring-sky-500/35");
    expect(card.querySelector(".tabler-icon-loader-2")).not.toBeNull();
    expect(card.querySelector("[data-retry-wait]")).toBeNull();
    expect(card.querySelector("[data-status-mark]")).toBeNull();
    expect(sessionChatSpy).not.toHaveBeenCalled();

    view.rerender(renderCard(true));

    expect(screen.getByTestId("node-session-chat")).toHaveAttribute(
      "data-session-id",
      "session-1",
    );
    expect(sessionChatSpy).toHaveBeenLastCalledWith(
      expect.objectContaining({ sessionId: "session-1", status: "running" }),
    );
    expect(screen.queryByText(SESSION_PENDING)).not.toBeInTheDocument();
    expect(screen.queryByText("Agent 正在处理")).not.toBeInTheDocument();
  });
});
