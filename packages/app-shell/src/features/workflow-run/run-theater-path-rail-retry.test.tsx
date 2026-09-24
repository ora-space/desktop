import { createRef } from "react";
import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  GraphWorkflowNodeState,
  GraphWorkflowRun,
  WorkflowNodeRetryWait,
} from "@ora/workflow-runtime";
import { AppI18nProvider } from "../../i18n/i18n";
import { appI18n } from "../../i18n/i18n-instance";
import { RunTheaterPathRail } from "./run-theater-path-rail";

const NOW = Date.UTC(2026, 8, 24, 6, 0, 0);

const WAIT: WorkflowNodeRetryWait = {
  attempt: 2,
  maxAttempt: 4,
  retry: 1,
  maxRetries: 3,
  delayMs: 30_000,
  scheduledAt: NOW,
  dueAt: NOW + 30_000,
};

const WAITING: GraphWorkflowNodeState = {
  status: "retry_waiting",
  retryWait: WAIT,
  autoRetry: { retry: 1, maxRetries: 3 },
};

beforeEach(async () => {
  await appI18n.changeLanguage("zh-CN");
  // Only the clock is frozen: countdowns stay at a fixed value while user events keep real timers.
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(NOW);
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

/** start (succeeded) -> fix (waiting to retry) -> out (idle). */
function linearRun(fixState: GraphWorkflowNodeState): GraphWorkflowRun {
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
          id: "start",
          type: "workflow",
          position: { x: 0, y: 0 },
          data: { kind: "start", title: "输入需求", description: "" },
        },
        {
          id: "fix",
          type: "workflow",
          position: { x: 300, y: 0 },
          data: { kind: "agent", title: "修复缺陷", description: "" },
        },
        {
          id: "out",
          type: "workflow",
          position: { x: 600, y: 0 },
          data: { kind: "output", title: "汇总输出", description: "" },
        },
      ],
      edges: [
        { id: "start-fix", source: "start", target: "fix" },
        { id: "fix-out", source: "fix", target: "out" },
      ],
    },
    name: "Retry run",
    status: "running",
    nodeStates: {
      start: { status: "succeeded" },
      fix: fixState,
      out: { status: "idle" },
    },
    openHitls: [],
    createdAt: "2026-09-24T14:00:00+08:00",
    updatedAt: "2026-09-24T14:00:00+08:00",
  };
}

/** start -> iter { body-a, body-b in parallel } -> out; body-b waits to retry in round 3. */
function iterationRun(): GraphWorkflowRun {
  return {
    id: "run-iteration",
    projectId: "project",
    definitionId: "definition",
    definitionSnapshot: {
      id: "snapshot",
      name: "Iteration run",
      description: "",
      updatedAt: "2026-09-24T14:00:00+08:00",
      viewport: { x: 0, y: 0, zoom: 1 },
      nodes: [
        {
          id: "start",
          type: "workflow",
          position: { x: 0, y: 160 },
          data: { kind: "start", title: "输入待处理项", description: "" },
        },
        {
          id: "iter",
          type: "workflow",
          position: { x: 300, y: 160 },
          data: { kind: "iteration", title: "逐项处理", description: "" },
        },
        {
          id: "body-a",
          type: "workflow",
          parentId: "iter",
          position: { x: 120, y: 80 },
          data: { kind: "agent", title: "Agent 1", description: "" },
        },
        {
          id: "body-b",
          type: "workflow",
          parentId: "iter",
          position: { x: 120, y: 240 },
          data: { kind: "agent", title: "生成处理结果", description: "" },
        },
        {
          id: "out",
          type: "workflow",
          position: { x: 1_200, y: 160 },
          data: { kind: "output", title: "汇总输出", description: "" },
        },
      ],
      edges: [
        { id: "start-iter", source: "start", target: "iter" },
        {
          id: "entry-a",
          source: "iter",
          sourceHandle: "iteration-entry",
          target: "body-a",
        },
        {
          id: "entry-b",
          source: "iter",
          sourceHandle: "iteration-entry",
          target: "body-b",
        },
        { id: "iter-out", source: "iter", target: "out" },
      ],
    },
    name: "Iteration run",
    status: "running",
    nodeStates: {
      start: { status: "succeeded" },
      iter: { status: "running" },
      "body-a": { status: "succeeded", iteration: 2 },
      "body-b": { ...WAITING, iteration: 2 },
      out: { status: "idle" },
    },
    roundStates: {
      "body-a": [0, 1, 2].map((iteration) => ({
        status: "succeeded" as const,
        iteration,
      })),
      "body-b": [
        {
          status: "succeeded",
          iteration: 0,
          startedAt: "2026-09-24T13:50:00+08:00",
          finishedAt: "2026-09-24T13:50:05+08:00",
        },
        {
          status: "succeeded",
          iteration: 1,
          startedAt: "2026-09-24T13:55:00+08:00",
          finishedAt: "2026-09-24T13:55:07+08:00",
        },
        { ...WAITING, iteration: 2 },
      ],
    },
    openHitls: [],
    createdAt: "2026-09-24T13:50:00+08:00",
    updatedAt: "2026-09-24T14:00:00+08:00",
  };
}

interface RailOptions {
  run: GraphWorkflowRun;
  primaryId: string | null;
  activeIds: string[];
  selectedRound?: number | null;
  onFocusNode?: (nodeId: string) => void;
  onExpandHitl?: (requestId: string) => void;
}

function rail({
  run,
  primaryId,
  activeIds,
  selectedRound = null,
  onFocusNode = vi.fn(),
  onExpandHitl = vi.fn(),
}: RailOptions) {
  return (
    <AppI18nProvider>
      <RunTheaterPathRail
        run={run}
        primaryId={primaryId}
        activeIds={activeIds}
        openHitls={[]}
        artifactCountByNode={{}}
        showResultAct={false}
        selectedRound={selectedRound}
        onRoundChange={vi.fn()}
        pathRailRef={createRef()}
        onFocusNode={onFocusNode}
        onExpandHitl={onExpandHitl}
      />
    </AppI18nProvider>
  );
}

describe("RunTheaterPathRail waiting retry chip", () => {
  it("does not count a waiting node as done and tints its chip orange with a countdown", () => {
    const view = render(
      rail({ run: linearRun(WAITING), primaryId: "start", activeIds: ["fix"] }),
    );

    expect(screen.getByText("1 / 3")).toBeInTheDocument();
    expect(screen.getByRole("progressbar", { name: "进度" })).toHaveAttribute(
      "aria-valuenow",
      "33",
    );

    const chip = screen.getByRole("button", {
      name: "修复缺陷: 等待重试（第 2/4 次）",
    });
    expect(chip).toHaveAttribute("data-retry-waiting", "");
    expect(chip).not.toHaveAttribute("data-waiting");
    expect(chip).not.toHaveAttribute("aria-current");
    expect(chip).toHaveClass(
      "border-orange-500/40",
      "bg-orange-500/10",
      "text-orange-950",
    );
    // Listed in activeIds, but the orange tint replaces the sky active cue.
    expect(chip).not.toHaveClass("border-sky-500/40");
    expect(chip).not.toHaveClass("bg-sky-500/10");
    const countdown = within(chip).getByText("30 秒后开始");
    expect(countdown).toHaveAttribute("data-retry-wait", "compact");
    // The countdown follows the title inside the chip.
    expect(chip).toHaveTextContent(/^修复缺陷30 秒后开始$/);
    expect(
      screen.getByRole("button", { name: "输入需求: 成功" }),
    ).not.toHaveAttribute("data-retry-waiting");

    // Regression: the same node running keeps the sky active cue and no countdown.
    view.rerender(
      rail({
        run: linearRun({
          status: "running",
          startedAt: "2026-09-24T14:00:00Z",
        }),
        primaryId: "start",
        activeIds: ["fix"],
      }),
    );
    const runningChip = screen.getByRole("button", {
      name: "修复缺陷: 运行中",
    });
    expect(runningChip).toHaveClass("border-sky-500/40", "bg-sky-500/10");
    expect(runningChip).not.toHaveAttribute("data-retry-waiting");
    expect(runningChip.querySelector("[data-retry-wait]")).toBeNull();
    expect(screen.getByText("1 / 3")).toBeInTheDocument();
  });

  it("marks a selected waiting chip with the stronger orange border and focuses it on click", async () => {
    const user = userEvent.setup();
    const onFocusNode = vi.fn();
    const onExpandHitl = vi.fn();
    render(
      rail({
        run: linearRun(WAITING),
        primaryId: "fix",
        activeIds: ["fix"],
        onFocusNode,
        onExpandHitl,
      }),
    );

    const chip = screen.getByRole("button", {
      name: "修复缺陷: 等待重试（第 2/4 次）",
    });
    expect(chip).toHaveAttribute("aria-current", "step");
    expect(chip).toHaveClass("theater-chip-pop", "border-orange-500/55");
    expect(chip).not.toHaveClass("border-orange-500/40");
    expect(chip).not.toHaveClass("border-foreground/35");
    expect(chip).not.toHaveClass("border-sky-500/40");
    expect(within(chip).getByText("30 秒后开始")).toBeInTheDocument();

    await user.click(chip);
    expect(onFocusNode).toHaveBeenCalledTimes(1);
    expect(onFocusNode).toHaveBeenCalledWith("fix");
    expect(onExpandHitl).not.toHaveBeenCalled();
  });
});

describe("RunTheaterRegionNavigator waiting retry member", () => {
  it("shows a countdown for a waiting member and leaves it out of the round progress", () => {
    const run = iterationRun();
    const view = render(
      rail({ run, primaryId: "body-a", activeIds: ["iter", "body-b"] }),
    );

    const navigator = screen.getByRole("region", {
      name: "逐项处理，第 3/3 轮",
    });
    expect(within(navigator).getByText("1/2 完成")).toBeInTheDocument();

    const member = within(navigator).getByRole("button", {
      name: "生成处理结果: 等待重试（第 2/4 次）",
    });
    expect(member).toHaveClass("border-orange-500/40");
    expect(member).not.toHaveClass("border-violet-500/45");
    expect(member).not.toHaveClass("border-border/65");
    const countdown = within(member).getByText("30 秒后开始");
    expect(countdown).toHaveAttribute("data-retry-wait", "compact");
    expect(member).toHaveTextContent(/^生成处理结果30 秒后开始$/);
    // The selected member keeps its violet border; only the waiting peer is orange.
    expect(
      within(navigator).getByRole("button", { name: "Agent 1: 成功" }),
    ).toHaveClass("border-violet-500/45");

    // An earlier round of the same member shows its finished result and duration instead.
    view.rerender(
      rail({
        run,
        primaryId: "body-a",
        activeIds: ["iter", "body-b"],
        selectedRound: 1,
      }),
    );
    const earlier = screen.getByRole("region", {
      name: "逐项处理，第 2/3 轮",
    });
    expect(within(earlier).getByText("2/2 完成")).toBeInTheDocument();
    const earlierMember = within(earlier).getByRole("button", {
      name: "生成处理结果: 成功",
    });
    expect(earlierMember).toHaveTextContent(/^生成处理结果7s$/);
    expect(earlierMember.querySelector("[data-retry-wait]")).toBeNull();
    expect(earlierMember).not.toHaveClass("border-orange-500/40");
  });

  it("keeps the violet selection border when the waiting member is selected", async () => {
    const user = userEvent.setup();
    const onFocusNode = vi.fn();
    render(
      rail({
        run: iterationRun(),
        primaryId: "body-b",
        activeIds: ["iter", "body-b"],
        onFocusNode,
      }),
    );

    const member = screen.getByRole("button", {
      name: "生成处理结果: 等待重试（第 2/4 次）",
    });
    expect(member).toHaveAttribute("aria-current", "step");
    expect(member).toHaveClass("border-violet-500/45");
    expect(member).not.toHaveClass("border-orange-500/40");
    expect(within(member).getByText("30 秒后开始")).toBeInTheDocument();

    await user.click(member);
    expect(onFocusNode).toHaveBeenCalledWith("body-b");
  });
});
