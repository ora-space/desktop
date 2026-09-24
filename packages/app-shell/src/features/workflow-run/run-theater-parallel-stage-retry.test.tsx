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
import { RunTheaterParallelStage } from "./run-theater-parallel-stage";

const NOW = Date.UTC(2026, 8, 24, 6, 0, 0);

const WAIT: WorkflowNodeRetryWait = {
  attempt: 3,
  maxAttempt: 4,
  retry: 2,
  maxRetries: 3,
  delayMs: 45_000,
  scheduledAt: NOW,
  dueAt: NOW + 45_000,
};

const RUNNING_DATA: WorkflowNodeData = {
  kind: "agent",
  title: "运行测试",
  description: "运行单元测试",
};

const WAITING_DATA: WorkflowNodeData = {
  kind: "agent",
  title: "代码评审",
  description: "评审当前分支",
};

const RUNNING_STATE: GraphWorkflowNodeState = {
  status: "running",
  sessionId: "session-tests",
  startedAt: "2026-09-24T13:59:00+08:00",
};

const WAITING_STATE: GraphWorkflowNodeState = {
  status: "retry_waiting",
  retryWait: WAIT,
  autoRetry: { retry: 2, maxRetries: 3 },
};

/** Application providers the act cards need (installed agent catalog). */
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

function stage(primaryId: string) {
  return (
    <RunTheaterParallelStage
      runId="run-parallel"
      acts={[
        {
          nodeId: "tests",
          data: RUNNING_DATA,
          state: RUNNING_STATE,
          artifactCount: 0,
          conversation: [],
        },
        {
          nodeId: "review",
          data: WAITING_DATA,
          state: WAITING_STATE,
          artifactCount: 0,
          conversation: [],
        },
      ]}
      primaryId={primaryId}
      onFocusNode={vi.fn()}
      inspectorOpen={false}
      onToggleInspector={vi.fn()}
    />
  );
}

beforeEach(async () => {
  await appI18n.changeLanguage("zh-CN");
  // Freeze only the clock so the countdown text is fixed; timers stay real.
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(NOW);
});

afterEach(async () => {
  cleanup();
  vi.useRealTimers();
  await appI18n.changeLanguage("zh-CN");
});

describe("RunTheaterParallelStage with a waiting retry act", () => {
  it("tints only the waiting act's chip and keeps its card off the live cue", () => {
    render(stage("tests"), { wrapper: createWrapper() });

    const waitingChip = screen.getByRole("button", {
      name: "聚焦到 代码评审: 等待重试（第 3/4 次）",
    });
    expect(waitingChip).toHaveAttribute("data-retry-waiting", "");
    expect(waitingChip).toHaveAttribute("aria-pressed", "false");
    expect(waitingChip).toHaveClass(
      "border-orange-500/40",
      "bg-orange-500/10",
      "text-orange-950",
    );

    expect(within(waitingChip).getByText("45 秒后开始")).toHaveAttribute(
      "data-retry-wait",
      "compact",
    );

    const runningChip = screen.getByRole("button", { name: "聚焦到 运行测试" });
    expect(runningChip).not.toHaveAttribute("data-retry-waiting");
    expect(runningChip.querySelector("[data-retry-wait]")).toBeNull();
    expect(runningChip).toHaveAttribute("aria-pressed", "true");
    expect(runningChip).toHaveClass("border-foreground/35");
    expect(runningChip.className).not.toContain("orange");

    const waitingCard = screen.getByLabelText("代码评审: 等待重试");
    expect(
      within(waitingCard).getByText("等待重试（第 3/4 次），45 秒后开始"),
    ).toHaveAttribute("data-retry-wait", "full");
    expect(
      waitingCard.querySelector('[data-status-mark="retry_waiting"]'),
    ).not.toBeNull();
    expect(waitingCard.querySelector(".tabler-icon-loader-2")).toBeNull();
    expect(waitingCard.className).not.toContain("theater-live-breathe");
    expect(waitingCard).not.toHaveClass("ring-sky-500/35");

    // Regression: the focused running act keeps its working cue.
    const runningCard = screen.getByLabelText("运行测试: 运行中");
    expect(runningCard).toHaveClass("theater-live-breathe", "ring-sky-500/35");
    expect(runningCard.querySelector(".tabler-icon-loader-2")).not.toBeNull();
    expect(runningCard.querySelector("[data-retry-wait]")).toBeNull();
  });

  it("uses the stronger orange chip when the waiting act is focused and still shows no live cue", () => {
    render(stage("review"), { wrapper: createWrapper() });

    const waitingChip = screen.getByRole("button", {
      name: "聚焦到 代码评审: 等待重试（第 3/4 次）",
    });
    expect(waitingChip).toHaveAttribute("aria-pressed", "true");
    expect(waitingChip).toHaveAttribute("data-retry-waiting", "");
    expect(waitingChip).toHaveClass("border-orange-500/55", "bg-orange-500/15");
    expect(waitingChip).not.toHaveClass("border-foreground/35");
    expect(waitingChip).not.toHaveClass("border-orange-500/40");

    const runningChip = screen.getByRole("button", { name: "聚焦到 运行测试" });
    expect(runningChip).not.toHaveAttribute("data-retry-waiting");
    expect(runningChip).toHaveClass("border-border/70", "bg-muted/40");

    const waitingCard = screen.getByLabelText("代码评审: 等待重试");
    expect(
      within(waitingCard).getByText("等待重试（第 3/4 次），45 秒后开始"),
    ).toBeInTheDocument();
    expect(waitingCard.querySelector(".tabler-icon-loader-2")).toBeNull();
    expect(document.querySelector(".theater-live-breathe")).toBeNull();
  });

  it("counts the chip down each second while its accessible name stays the same", async () => {
    vi.useRealTimers();
    vi.useFakeTimers();
    vi.setSystemTime(NOW);
    render(stage("tests"), { wrapper: createWrapper() });
    const name = "聚焦到 代码评审: 等待重试（第 3/4 次）";
    expect(
      within(screen.getByRole("button", { name })).getByText("45 秒后开始"),
    ).toBeInTheDocument();

    for (let second = 0; second < 3; second += 1) {
      await act(async () => {
        await vi.advanceTimersByTimeAsync(1_000);
      });
    }
    expect(
      within(screen.getByRole("button", { name })).getByText("42 秒后开始"),
    ).toBeInTheDocument();
  });

  it("names the attempt in English", async () => {
    await appI18n.changeLanguage("en-US");
    render(stage("tests"), { wrapper: createWrapper() });
    const chip = screen.getByRole("button", {
      name: "Focus 代码评审: Waiting to retry (attempt 3/4)",
    });
    expect(within(chip).getByText("starts in 45s")).toBeInTheDocument();
  });
});
