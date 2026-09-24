import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createChatStore } from "@ora/chat";
import {
  createTestClient,
  type TestHandlers,
} from "../../test/contracts-transport";
import { createPluginMemory, pluginHandlers } from "../../test/memory/plugins";
import { createAgentMemory, agentHandlers } from "../../test/memory/agents";
import { createSkillMemory, skillHandlers } from "../../test/memory/skills";
import {
  createHookWrapper,
  createTestQueryClient,
} from "../../test/hook-harness";
import { formatRunClock } from "../../lib/format";
import { appI18n } from "../../i18n/i18n-instance";
import { RunActInspector } from "./run-act-inspector";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import type {
  GraphWorkflowNodeState,
  GraphWorkflowRunStatus,
  WorkflowNodeAttemptFailure,
  WorkflowNodeData,
  WorkflowNodeRetryWait,
} from "@ora/workflow-runtime";
import { AGENT_REF } from "../../test/agent-identity";

/** State for this test surface; no unrelated domain fixtures are initialized. */
function createFixtureState() {
  return {
    ...createPluginMemory(),
    ...createAgentMemory(),
    ...createSkillMemory(),
  };
}

type FixtureState = ReturnType<typeof createFixtureState>;

/** Explicit domain composition for the behaviors exercised by this test file. */
function createFixtureHandlers(state: FixtureState): TestHandlers {
  return {
    ...pluginHandlers(state),
    ...agentHandlers(state),
    ...skillHandlers(state),
  };
}

const AGENT_DATA: WorkflowNodeData = {
  kind: "agent",
  title: "探索",
  description: "只读探索项目现状",
  agentConfig: {
    schemaVersion: 3,
    executor: {
      agentCli: AGENT_REF.opencode,
      modelId: "deepseek/deepseek-v4-pro",
    },
    roleId: "研究员",
    skills: [{ skillId: "openspec-explore", enabled: true }],
    mcps: [],
    prompt: "阅读相关代码并输出风险。",
  },
};

/** Mounts the act inspector with catalog-backed Agent/Skill names, as the main inspector test does. */
function renderInspector(
  nodeState: GraphWorkflowNodeState,
  options: {
    runStatus?: GraphWorkflowRunStatus;
    roundStates?: Record<string, GraphWorkflowNodeState[]>;
    selectedRound?: number | null;
    data?: WorkflowNodeData;
  } = {},
) {
  useWorkspaceSelectionStore.getState().selectWorkflowRun("run-1", "project-1");
  const state = createFixtureState();
  state.agents = [
    {
      id: "ag-researcher",
      namespace: "local",
      name: "研究员",
      description: "只读探索项目现状和影响范围",
    },
  ];
  state.skills = [
    {
      id: "sk-explore",
      namespace: "local",
      name: "openspec-explore",
      description: "探索仓库结构与约束",
      source: { kind: "local" } as const,
      availability: "available",
    },
  ];
  const client = createTestClient(createFixtureHandlers(state));
  const queryClient = createTestQueryClient();
  const Wrapper = createHookWrapper(
    client,
    queryClient,
    createChatStore(client.session),
  );

  return {
    user: userEvent.setup(),
    ...render(
      <Wrapper>
        <RunActInspector
          nodeId="agent-1"
          data={options.data ?? AGENT_DATA}
          state={nodeState}
          roundStates={options.roundStates}
          selectedRound={options.selectedRound ?? null}
          onRoundChange={() => undefined}
          artifacts={[]}
          revealedArtifactId={null}
          onClose={() => undefined}
          runStatus={options.runStatus}
        />
      </Wrapper>,
    ),
  };
}

/** Waits for the catalog-backed agent summary so query updates settle inside the test. */
async function settled() {
  expect(
    await screen.findByText("OpenCode · deepseek/deepseek-v4-pro"),
  ).toBeInTheDocument();
}

/** True when `first` comes before `second` in document order. */
function precedes(first: Node, second: Node): boolean {
  return (
    (first.compareDocumentPosition(second) &
      Node.DOCUMENT_POSITION_FOLLOWING) !==
    0
  );
}

const NOW = Date.UTC(2026, 8, 24, 6, 0, 0);
const STARTED_AT = "2026-09-24T05:58:10.000Z";
const FINISHED_AT = "2026-09-24T05:59:02.000Z";

const RETRY_WAIT: WorkflowNodeRetryWait = {
  attempt: 2,
  maxAttempt: 4,
  retry: 1,
  maxRetries: 3,
  delayMs: 3_000,
  scheduledAt: NOW,
  dueAt: NOW + 3_000,
};

const WAITING_STATE: GraphWorkflowNodeState = {
  status: "retry_waiting",
  retryWait: RETRY_WAIT,
  autoRetry: { retry: 1, maxRetries: 3 },
  // The adapter never sets these for a waiting row; the inspector must still ignore them.
  startedAt: STARTED_AT,
  finishedAt: FINISHED_AT,
};

const RESUME_HINT = "可在顶部点「从失败处继续」重跑这个节点";

beforeEach(async () => {
  await appI18n.changeLanguage("zh-CN");
});

afterEach(async () => {
  // Unmount first: switching the language re-renders every mounted useTranslation consumer
  // outside act.
  cleanup();
  vi.useRealTimers();
  await appI18n.changeLanguage("zh-CN");
});

describe("RunActInspector waiting retry", () => {
  beforeEach(() => {
    // Freeze only the clock the countdown reads; query and user-event timers stay real.
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(NOW);
  });

  it("shows the live waiting label under the description and the orange status badge", async () => {
    const { container } = renderInspector(WAITING_STATE, {
      runStatus: "running",
    });
    await settled();

    const description = screen.getByText("只读探索项目现状");
    const label = description.nextElementSibling;
    expect(label?.textContent).toBe("等待重试（第 2/4 次），3 秒后开始");
    expect(label?.querySelector('[data-retry-wait="full"]')).not.toBeNull();
    expect(container.querySelectorAll("[data-retry-wait]")).toHaveLength(1);

    const badge = screen.getByText("等待重试");
    expect(badge.className).toContain("border-orange-500/30");
    expect(badge.querySelector(".bg-orange-500")).not.toBeNull();
    // The header badge is quiet: a dot, not the retry glyph mark.
    expect(badge.querySelector("[data-status-mark]")).toBeNull();
  });

  it("hides the time range, error, retry notes, resume hint and AI diagnosis while waiting", async () => {
    renderInspector(WAITING_STATE, { runStatus: "running" });
    await settled();

    const range = `${formatRunClock(STARTED_AT, "zh-CN")} — ${formatRunClock(FINISHED_AT, "zh-CN")}`;
    expect(screen.queryByText(range)).not.toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(screen.queryByText(/仍然失败/u)).not.toBeInTheDocument();
    expect(screen.queryByText(RESUME_HINT)).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "让 AI 分析" }),
    ).not.toBeInTheDocument();
    expect(screen.queryByText("之前失败的尝试")).not.toBeInTheDocument();
  });

  it("shows the same time range once the node is no longer waiting", async () => {
    renderInspector(
      { status: "succeeded", startedAt: STARTED_AT, finishedAt: FINISHED_AT },
      { runStatus: "running" },
    );
    await settled();

    expect(
      screen.getByText(
        `${formatRunClock(STARTED_AT, "zh-CN")} — ${formatRunClock(FINISHED_AT, "zh-CN")}`,
      ),
    ).toBeInTheDocument();
    expect(document.querySelector("[data-retry-wait]")).toBeNull();
  });

  it("marks a waiting round's tab dot orange", async () => {
    const round0: GraphWorkflowNodeState = {
      status: "succeeded",
      iteration: 0,
    };
    const round1: GraphWorkflowNodeState = {
      ...WAITING_STATE,
      iteration: 1,
    };
    renderInspector(round1, {
      runStatus: "running",
      selectedRound: 1,
      roundStates: { "agent-1": [round0, round1] },
    });
    await settled();

    const tabs = screen.getAllByRole("tab");
    expect(tabs.map((tab) => tab.textContent)).toEqual(["R1", "R2"]);
    const waitingDot = tabs[1]!.querySelector("span");
    expect(waitingDot?.className).toContain("bg-orange-500");
    expect(waitingDot?.className).not.toContain("theater-live-breathe");
    expect(tabs[0]!.querySelector("span")?.className).toContain(
      "bg-emerald-500",
    );
  });
});

describe("RunActInspector exhausted automatic retries", () => {
  // The failed row ran: it has a start time, so the retry that scheduled it counts.
  const failedAfterRetries = (retry: number): GraphWorkflowNodeState => ({
    status: "failed",
    errorMessage: "agent session ended with an error",
    startedAt: "2026-09-24T06:00:00.000Z",
    finishedAt: "2026-09-24T06:00:30.000Z",
    autoRetry: { retry, maxRetries: 3 },
  });

  it("says how many automatic retries ran, before the resume hint", async () => {
    renderInspector(failedAfterRetries(2), { runStatus: "failed" });
    await settled();

    const note = screen.getByText("已自动重试 2 次，仍然失败");
    const hint = screen.getByText(RESUME_HINT);
    expect(precedes(note, hint)).toBe(true);
    expect(
      screen.getByRole("button", { name: "让 AI 分析" }),
    ).toBeInTheDocument();
  });

  it.each([
    { retry: 1, text: "Retried automatically 1 time, still failed" },
    { retry: 2, text: "Retried automatically 2 times, still failed" },
  ])("uses the English plural for $retry retries", async ({ retry, text }) => {
    await appI18n.changeLanguage("en-US");
    renderInspector(failedAfterRetries(retry), { runStatus: "failed" });
    await settled();

    expect(screen.getByText(text)).toBeInTheDocument();
  });

  it("shows no retry note without an automatic retry or with retry 0", async () => {
    const plain = renderInspector(
      { status: "failed", errorMessage: "agent session ended with an error" },
      { runStatus: "failed" },
    );
    await settled();
    expect(screen.queryByText(/已自动重试/u)).not.toBeInTheDocument();
    expect(screen.getByText(RESUME_HINT)).toBeInTheDocument();
    plain.unmount();

    renderInspector(failedAfterRetries(0), { runStatus: "failed" });
    await settled();
    expect(screen.queryByText(/已自动重试/u)).not.toBeInTheDocument();
    expect(screen.getByText(RESUME_HINT)).toBeInTheDocument();
  });
});

describe("RunActInspector failure kinds that are never retried", () => {
  const NOT_RETRIED = "这类失败不会自动重试";

  function failedWith(
    autoRetryable: boolean | undefined,
  ): GraphWorkflowNodeState {
    return {
      status: "failed",
      errorMessage: "prompt references a missing variable",
      errorDetail: {
        kind: "prompt_template",
        message: "prompt references a missing variable",
        sourceChain: [],
        attempt: 1,
        resumable: false,
        injectsPreviousFailure: false,
        ...(autoRetryable !== undefined ? { autoRetryable } : {}),
        recordedAt: 50,
      },
      finishedAt: "2026-09-24T06:01:00.000Z",
    };
  }

  function agentWith(
    extra: Partial<NonNullable<WorkflowNodeData["agentConfig"]>>,
  ): WorkflowNodeData {
    return {
      ...AGENT_DATA,
      agentConfig: { ...AGENT_DATA.agentConfig!, ...extra },
    };
  }

  it("explains the immediate failure when the node's retry policy is on", async () => {
    const { unmount } = renderInspector(failedWith(false), {
      runStatus: "failed",
    });
    await settled();
    expect(screen.getByRole("alert")).toHaveTextContent(NOT_RETRIED);
    unmount();

    renderInspector(failedWith(false), {
      runStatus: "failed",
      data: agentWith({
        retry: { enabled: true, maxRetries: 3, initialDelaySeconds: 5 },
      }),
    });
    await settled();
    expect(screen.getByText(NOT_RETRIED)).toBeInTheDocument();
  });

  it("stays silent when retry is off, has no budget, or the node is interactive", async () => {
    for (const data of [
      agentWith({
        retry: { enabled: false, maxRetries: 2, initialDelaySeconds: 10 },
      }),
      agentWith({
        retry: { enabled: true, maxRetries: 0, initialDelaySeconds: 10 },
      }),
      agentWith({ interactive: true }),
    ]) {
      const { unmount } = renderInspector(failedWith(false), {
        runStatus: "failed",
        data,
      });
      await settled();
      expect(screen.queryByText(NOT_RETRIED)).not.toBeInTheDocument();
      unmount();
    }
  });

  it("stays silent for a retryable kind and for rows without the recorded flag", async () => {
    for (const autoRetryable of [true, undefined]) {
      const { unmount } = renderInspector(failedWith(autoRetryable), {
        runStatus: "failed",
      });
      await settled();
      expect(screen.queryByText(NOT_RETRIED)).not.toBeInTheDocument();
      unmount();
    }
  });

  it("uses the English copy in the English UI", async () => {
    // The file-level afterEach restores zh-CN after unmounting; switching back here would
    // re-render the mounted inspector outside act.
    await appI18n.changeLanguage("en-US");
    renderInspector(failedWith(false), { runStatus: "failed" });
    await settled();

    expect(
      screen.getByText("This kind of failure is not retried automatically"),
    ).toBeInTheDocument();
  });
});

describe("RunActInspector retries that never started", () => {
  const NOT_STARTED = "这次自动重试已安排，但没有开始";

  it("does not count a retry whose wait an app restart ended as a retry that ran", async () => {
    renderInspector(
      {
        status: "failed",
        errorMessage: '{"reason":"interrupted_by_restart"}',
        errorDetail: {
          kind: "interrupted_by_restart",
          message: '{"reason":"interrupted_by_restart"}',
          sourceChain: [],
          attempt: 2,
          resumable: true,
          injectsPreviousFailure: false,
          recordedAt: 50,
        },
        finishedAt: "2026-09-24T06:01:00.000Z",
        autoRetry: { retry: 1, maxRetries: 3 },
      },
      { runStatus: "failed" },
    );
    await settled();

    expect(screen.queryByText(/已自动重试/u)).not.toBeInTheDocument();
    expect(screen.getByText(NOT_STARTED)).toBeInTheDocument();
    expect(screen.getByText(RESUME_HINT)).toBeInTheDocument();
  });

  it("counts only the retries that started when the last one never did", async () => {
    renderInspector(
      {
        status: "failed",
        errorMessage: '{"reason":"interrupted_by_restart"}',
        finishedAt: "2026-09-24T06:01:00.000Z",
        autoRetry: { retry: 2, maxRetries: 3 },
      },
      { runStatus: "failed" },
    );
    await settled();

    expect(screen.getByText("已自动重试 1 次，仍然失败")).toBeInTheDocument();
    expect(screen.getByText(NOT_STARTED)).toBeInTheDocument();
  });

  it("says a retry cancelled while waiting never started", async () => {
    renderInspector(
      {
        status: "cancelled",
        finishedAt: "2026-09-24T06:01:00.000Z",
        autoRetry: { retry: 1, maxRetries: 3 },
      },
      { runStatus: "cancelled" },
    );
    await settled();

    expect(screen.getByText(NOT_STARTED)).toBeInTheDocument();
    expect(screen.queryByText(/已自动重试/u)).not.toBeInTheDocument();
  });

  it("uses only the abandoned note for a wait the run abandoned", async () => {
    renderInspector(
      {
        status: "cancelled",
        errorMessage: '{"reason":"retry_abandoned"}',
        retryAbandoned: true,
        finishedAt: "2026-09-24T06:01:00.000Z",
        autoRetry: { retry: 1, maxRetries: 3 },
      },
      { runStatus: "failed" },
    );
    await settled();

    expect(
      screen.getByText("运行在等待自动重试时结束，这次重试没有开始"),
    ).toBeInTheDocument();
    expect(screen.queryByText(NOT_STARTED)).not.toBeInTheDocument();
    expect(screen.queryByText(/已自动重试/u)).not.toBeInTheDocument();
  });

  it("shows neither note while the retry is still waiting", async () => {
    renderInspector(
      {
        status: "retry_waiting",
        autoRetry: { retry: 1, maxRetries: 3 },
        retryWait: {
          attempt: 2,
          maxAttempt: 4,
          retry: 1,
          maxRetries: 3,
          delayMs: 10_000,
          scheduledAt: Date.now(),
          dueAt: Date.now() + 600_000,
        },
      },
      { runStatus: "running" },
    );
    await settled();

    expect(screen.queryByText(NOT_STARTED)).not.toBeInTheDocument();
    expect(screen.queryByText(/仍然失败/u)).not.toBeInTheDocument();
  });
});

describe("RunActInspector abandoned retry", () => {
  it("replaces the raw abandoned error with a note", async () => {
    renderInspector(
      {
        status: "cancelled",
        errorMessage: '{"reason":"retry_abandoned"}',
        retryAbandoned: true,
        autoRetry: { retry: 1, maxRetries: 2 },
      },
      { runStatus: "failed" },
    );
    await settled();

    expect(
      screen.getByText("运行在等待自动重试时结束，这次重试没有开始"),
    ).toBeInTheDocument();
    expect(screen.queryByText(/retry_abandoned/u)).not.toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    // The exhausted note belongs to failed attempts only.
    expect(screen.queryByText(/仍然失败/u)).not.toBeInTheDocument();
  });
});

describe("RunActInspector earlier failed attempts", () => {
  const FAILED_ATTEMPTS: WorkflowNodeAttemptFailure[] = [
    {
      nodeRunId: "node-run-1",
      attempt: 1,
      kind: "session",
      errorMessage: "first attempt: connection reset",
      sourceChain: ["agent node failed", "connection reset"],
      recordedAt: NOW - 60_000,
      replacedBy: "automatic_retry",
    },
    {
      nodeRunId: "node-run-2",
      attempt: 2,
      kind: "structured_output",
      errorMessage: "second attempt: not json",
      sourceChain: ["not json"],
      recordedAt: NOW - 30_000,
      replacedBy: "automatic_retry",
    },
  ];

  const CURRENT: GraphWorkflowNodeState = {
    status: "failed",
    errorMessage: "third attempt: session crashed",
    errorDetail: {
      kind: "session",
      message: "third attempt: session crashed",
      sourceChain: [],
      attempt: 3,
      resumable: true,
      injectsPreviousFailure: true,
      recordedAt: 50,
    },
    injectedFailureContext:
      "## 上一次尝试（第 2 次）失败信息\n类型：结构化输出不合格",
    startedAt: "2026-09-24T06:00:00.000Z",
    finishedAt: "2026-09-24T06:00:30.000Z",
    autoRetry: { retry: 2, maxRetries: 2 },
  };

  it("lists earlier attempts after the current attempt's error block and injected context", async () => {
    renderInspector(
      { ...CURRENT, failedAttempts: FAILED_ATTEMPTS },
      { runStatus: "failed" },
    );
    await settled();

    const alert = screen.getByRole("alert");
    expect(within(alert).getByText("第 3 次尝试")).toBeInTheDocument();
    expect(within(alert).getByText("智能体会话失败")).toBeInTheDocument();
    expect(
      within(alert).getByText("third attempt: session crashed"),
    ).toBeInTheDocument();
    expect(screen.getByText("已自动重试 2 次，仍然失败")).toBeInTheDocument();

    const injected = screen.getByText("本次尝试注入的上次失败信息");
    expect(injected.tagName).toBe("SUMMARY");
    expect(injected.parentElement?.querySelector("pre")?.textContent).toBe(
      "## 上一次尝试（第 2 次）失败信息\n类型：结构化输出不合格",
    );

    const section = document.querySelector<HTMLElement>(
      '[data-slot="failed-attempts"]',
    );
    expect(section).not.toBeNull();
    expect(within(section!).getByText("之前失败的尝试")).toBeInTheDocument();
    expect(
      Array.from(section!.querySelectorAll("li[data-attempt]")).map((item) =>
        item.getAttribute("data-attempt"),
      ),
    ).toEqual(["1", "2"]);
    expect(
      within(section!).getByText("first attempt: connection reset"),
    ).toBeInTheDocument();
    expect(
      within(section!).getByText("second attempt: not json"),
    ).toBeInTheDocument();
    expect(within(section!).queryByText("第 3 次尝试")).not.toBeInTheDocument();

    expect(precedes(alert, section!)).toBe(true);
    expect(precedes(injected, section!)).toBe(true);
  });

  it("renders no earlier-attempts section when the state has none", async () => {
    renderInspector(CURRENT, { runStatus: "failed" });
    await settled();

    expect(
      screen.getByText("third attempt: session crashed"),
    ).toBeInTheDocument();
    expect(document.querySelector('[data-slot="failed-attempts"]')).toBeNull();
    expect(screen.queryByText("之前失败的尝试")).not.toBeInTheDocument();
  });

  it("lists earlier attempts while the node waits for its next retry", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(NOW);
    renderInspector(
      { ...WAITING_STATE, failedAttempts: [FAILED_ATTEMPTS[0]!] },
      { runStatus: "running" },
    );
    await settled();

    const section = document.querySelector<HTMLElement>(
      '[data-slot="failed-attempts"]',
    );
    expect(
      within(section!).getByText("first attempt: connection reset"),
    ).toBeInTheDocument();
    expect(within(section!).getByText("已自动重试")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });
});
