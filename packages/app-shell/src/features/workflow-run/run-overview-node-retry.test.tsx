import {
  afterAll,
  afterEach,
  beforeEach,
  describe,
  expect,
  it,
  vi,
} from "vitest";
import { cleanup, render, screen, within } from "@testing-library/react";
import { ReactFlow, ReactFlowProvider, type Node } from "@xyflow/react";
import type {
  GraphWorkflowNodeState,
  GraphWorkflowRun,
  WorkflowNodeRetryWait,
} from "@ora/workflow-runtime";
import { AppI18nProvider } from "../../i18n/i18n";
import { appI18n } from "../../i18n/i18n-instance";
import { formatRunClock } from "../../lib/format";
import { RunOverviewCanvas } from "./run-overview-canvas";
import {
  RunOverviewNode,
  type RunOverviewNodeData,
  RunOverviewStatusProvider,
} from "./run-overview-node";

const NOW = Date.UTC(2026, 8, 24, 6, 0, 0);
const STARTED_AT = "2026-09-24T13:59:10+08:00";

const WAIT: WorkflowNodeRetryWait = {
  attempt: 2,
  maxAttempt: 4,
  retry: 1,
  maxRetries: 3,
  delayMs: 20_000,
  scheduledAt: NOW,
  dueAt: NOW + 20_000,
};

const WAITING: GraphWorkflowNodeState = {
  status: "retry_waiting",
  retryWait: WAIT,
  autoRetry: { retry: 1, maxRetries: 3 },
};

// Stable node type map: React Flow warns when it changes between renders.
const NODE_TYPES = { workflow: RunOverviewNode };

const FLOW_NODES: Node<RunOverviewNodeData, "workflow">[] = [
  ["done", "整理需求", 0],
  ["fix", "修复缺陷", 320],
  ["check", "运行检查", 640],
].map(([id, title, x]) => ({
  id: id as string,
  type: "workflow",
  position: { x: x as number, y: 0 },
  data: {
    kind: "agent",
    title: title as string,
    description: "",
    runStatus: "idle",
  },
}));

/** Renders the Overview node inside a real React Flow with explicit status context. */
function renderNodes({
  states,
  focusedNodeId,
  activeNodeIds,
}: {
  states: Record<string, GraphWorkflowNodeState>;
  focusedNodeId: string | null;
  activeNodeIds: string[];
}) {
  return render(
    <AppI18nProvider>
      <div style={{ width: 1200, height: 800 }}>
        <ReactFlowProvider>
          <RunOverviewStatusProvider
            states={states}
            focusedNodeId={focusedNodeId}
            activeNodeIds={activeNodeIds}
            artifactCountByNode={{}}
          >
            <ReactFlow
              nodes={FLOW_NODES}
              edges={[]}
              nodeTypes={NODE_TYPES}
              proOptions={{ hideAttribution: true }}
            />
          </RunOverviewStatusProvider>
        </ReactFlowProvider>
      </div>
    </AppI18nProvider>,
  );
}

/** done (succeeded) -> fix (waiting to retry) and check (running) in parallel. */
function parallelRun(): GraphWorkflowRun {
  return {
    id: "run-retry",
    projectId: "project",
    definitionId: "definition",
    definitionSnapshot: {
      id: "snapshot",
      name: "Retry run",
      description: "",
      updatedAt: "2026-09-24T14:00:00+08:00",
      viewport: { x: 0, y: 0, zoom: 1 },
      nodes: [
        {
          id: "done",
          type: "workflow",
          position: { x: 0, y: 100 },
          data: { kind: "agent", title: "整理需求", description: "" },
        },
        {
          id: "fix",
          type: "workflow",
          position: { x: 320, y: 0 },
          data: { kind: "agent", title: "修复缺陷", description: "" },
        },
        {
          id: "check",
          type: "workflow",
          position: { x: 320, y: 220 },
          data: { kind: "agent", title: "运行检查", description: "" },
        },
      ],
      edges: [
        { id: "done-fix", source: "done", target: "fix" },
        { id: "done-check", source: "done", target: "check" },
      ],
    },
    name: "Retry run",
    status: "running",
    nodeStates: {
      done: {
        status: "succeeded",
        startedAt: "2026-09-24T13:58:00+08:00",
        finishedAt: "2026-09-24T13:58:30+08:00",
      },
      fix: WAITING,
      check: { status: "running", startedAt: STARTED_AT },
    },
    openHitls: [],
    createdAt: "2026-09-24T13:58:00+08:00",
    updatedAt: "2026-09-24T14:00:00+08:00",
  };
}

beforeEach(async () => {
  await appI18n.changeLanguage("zh-CN");
  // React Flow needs a sized container in jsdom.
  Object.defineProperty(HTMLElement.prototype, "clientWidth", {
    configurable: true,
    get: () => 1200,
  });
  Object.defineProperty(HTMLElement.prototype, "clientHeight", {
    configurable: true,
    get: () => 800,
  });
  // Freeze only the clock so the countdown text is fixed; timers stay real.
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(NOW);
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

afterAll(() => {
  // Drop the overrides so jsdom's own Element getters apply again.
  Reflect.deleteProperty(HTMLElement.prototype, "clientWidth");
  Reflect.deleteProperty(HTMLElement.prototype, "clientHeight");
});

describe("RunOverviewNode while waiting to retry", () => {
  it("shows the full waiting label instead of the time range in the footer", async () => {
    renderNodes({
      // A waiting row carrying stale timing still shows only the countdown.
      states: {
        done: { status: "succeeded" },
        fix: { ...WAITING, startedAt: STARTED_AT },
        check: { status: "idle" },
      },
      focusedNodeId: null,
      activeNodeIds: ["fix"],
    });

    const card = await screen.findByLabelText("修复缺陷: 等待重试");
    const label = within(card).getByText("等待重试（第 2/4 次），20 秒后开始");
    expect(label).toHaveAttribute("data-retry-wait", "full");
    expect(label.parentElement).toHaveClass("text-orange-700");
    expect(card).not.toHaveTextContent(formatRunClock(STARTED_AT, "zh-CN"));
    expect(card).not.toHaveTextContent("—");

    const badge = within(card).getByText("等待重试");
    expect(badge).toHaveClass("border-orange-500/30", "bg-orange-500/10");
    expect(
      badge.querySelector('[data-status-mark="retry_waiting"]'),
    ).not.toBeNull();
    expect(card.querySelector(".tabler-icon-loader-2")).toBeNull();
    expect(card.className).not.toContain("theater-live-breathe");
  });

  it("gives a waiting peer no sky ring while a succeeded peer keeps it", async () => {
    renderNodes({
      states: {
        done: { status: "succeeded" },
        fix: WAITING,
        check: { status: "running", startedAt: STARTED_AT },
      },
      focusedNodeId: "check",
      activeNodeIds: ["done", "fix", "check"],
    });

    const waitingCard = await screen.findByLabelText("修复缺陷: 等待重试");
    expect(waitingCard).not.toHaveClass("ring-sky-500/20");
    expect(waitingCard).toHaveClass(
      "border-orange-500/45",
      "ring-orange-500/15",
    );

    const succeededPeer = screen.getByLabelText("整理需求: 成功");
    expect(succeededPeer).toHaveClass("ring-sky-500/20");

    const focusedRunning = screen.getByLabelText("运行检查: 运行中");
    expect(focusedRunning).toHaveClass(
      "ring-sky-500/35",
      "theater-live-breathe",
    );
    expect(focusedRunning).toHaveTextContent(
      `${formatRunClock(STARTED_AT, "zh-CN")} — —`,
    );
  });

  it("keeps a waiting node listed as active without the sky ring on the run canvas", async () => {
    render(
      <AppI18nProvider>
        <div style={{ width: 1200, height: 800 }}>
          <RunOverviewCanvas
            run={parallelRun()}
            focusedNodeId="check"
            onFocusNode={vi.fn()}
          />
        </div>
      </AppI18nProvider>,
    );

    const waitingCard = await screen.findByLabelText("修复缺陷: 等待重试");
    expect(
      within(waitingCard).getByText("等待重试（第 2/4 次），20 秒后开始"),
    ).toBeInTheDocument();
    expect(waitingCard).not.toHaveClass("ring-sky-500/20");
    expect(waitingCard).not.toHaveClass("ring-sky-500/35");
    expect(waitingCard.className).not.toContain("theater-live-breathe");
    expect(within(waitingCard).getByText("等待重试")).toBeInTheDocument();
    expect(screen.getByLabelText("整理需求: 成功")).not.toHaveClass(
      "ring-sky-500/20",
    );
  });
});
