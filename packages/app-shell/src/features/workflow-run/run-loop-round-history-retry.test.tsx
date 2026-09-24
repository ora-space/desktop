import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  GraphWorkflowRound,
  WorkflowNodeRetryWait,
} from "@ora/workflow-runtime";
import { AppI18nProvider } from "../../i18n/i18n";
import { appI18n } from "../../i18n/i18n-instance";
import { RunLoopRoundHistory } from "./run-loop-round-history";

const NOW = Date.UTC(2026, 8, 24, 6, 0, 0);

const WAIT: WorkflowNodeRetryWait = {
  attempt: 2,
  maxAttempt: 3,
  retry: 1,
  maxRetries: 2,
  delayMs: 12_000,
  scheduledAt: NOW,
  dueAt: NOW + 12_000,
};

const ROUNDS: GraphWorkflowRound[] = [
  {
    id: "round-1",
    parentLoopNodeRunId: "loop-run",
    parentLoopNodeId: "loop",
    roundIndex: 0,
    status: "succeeded",
    nodeStates: {
      writer: { status: "succeeded", sessionId: "session-w1" },
      reviewer: { status: "failed", sessionId: "session-r1" },
    },
    createdAt: "2026-09-24T05:50:00.000Z",
    updatedAt: "2026-09-24T05:55:00.000Z",
  },
  {
    id: "round-2",
    parentLoopNodeRunId: "loop-run",
    parentLoopNodeId: "loop",
    roundIndex: 1,
    status: "running",
    nodeStates: {
      writer: { status: "succeeded", sessionId: "session-w2" },
      reviewer: {
        status: "retry_waiting",
        retryWait: WAIT,
        autoRetry: { retry: 1, maxRetries: 2 },
      },
    },
    createdAt: "2026-09-24T05:56:00.000Z",
    updatedAt: "2026-09-24T06:00:00.000Z",
  },
];

/** The row element of one node inside the selected round. */
function row(title: string): HTMLElement {
  const element = screen.getByText(title).parentElement;
  if (element === null) {
    throw new Error(`no row for ${title}`);
  }
  return element;
}

beforeEach(async () => {
  await appI18n.changeLanguage("zh-CN");
  // Freeze only the clock so the countdown text is fixed; timers stay real.
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(NOW);
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("RunLoopRoundHistory with a waiting retry", () => {
  it("shows the compact countdown and waiting badge on the waiting row only", async () => {
    const user = userEvent.setup();
    render(
      <AppI18nProvider>
        <RunLoopRoundHistory
          rounds={ROUNDS}
          nodeTitles={{ writer: "撰写", reviewer: "评审" }}
        />
      </AppI18nProvider>,
    );

    expect(screen.getByRole("tab", { name: "第 2 轮" })).toHaveAttribute(
      "aria-selected",
      "true",
    );

    const waitingRow = row("评审");
    expect(waitingRow.textContent).toBe(
      "评审等待重试（第 2/3 次）12 秒后开始等待重试",
    );
    // Screen readers get the attempt count, which the compact countdown leaves out.
    expect(within(waitingRow).getByText("等待重试（第 2/3 次）")).toHaveClass(
      "sr-only",
    );
    expect(within(waitingRow).getByText("12 秒后开始")).toHaveAttribute(
      "data-retry-wait",
      "compact",
    );
    const badge = within(waitingRow).getByText("等待重试");
    expect(badge).toHaveClass("border-orange-500/30", "bg-orange-500/10");
    // Quiet badge: the orange dot, never a spinner.
    expect(badge.querySelector(".bg-orange-500")).not.toBeNull();
    expect(badge.querySelector("svg")).toBeNull();
    expect(waitingRow.querySelector("[title]")).toBeNull();

    const writerRow = row("撰写");
    expect(writerRow).toHaveTextContent(/^撰写session-w2成功$/);
    expect(within(writerRow).getByTitle("session-w2")).toBeInTheDocument();
    expect(writerRow.querySelector("[data-retry-wait]")).toBeNull();
    expect(writerRow.querySelector(".sr-only")).toBeNull();

    // The same node in an earlier round shows its own failed state and session.
    await user.click(screen.getByRole("tab", { name: "第 1 轮" }));
    const failedRow = row("评审");
    expect(failedRow).toHaveTextContent(/^评审session-r1失败$/);
    expect(failedRow.querySelector("[data-retry-wait]")).toBeNull();
    expect(failedRow.querySelector(".sr-only")).toBeNull();
    expect(row("撰写")).toHaveTextContent(/^撰写session-w1成功$/);
  });
});
