import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { WorkflowNodeAttemptFailure } from "@ora/workflow-runtime";
import { formatRunClock } from "../../lib/format";
import { AppI18nProvider } from "../../i18n/i18n";
import { appI18n } from "../../i18n/i18n-instance";
import { RunActFailedAttempts } from "./run-act-failed-attempts";

const FIRST_STARTED = "2026-09-24T06:00:05.000Z";
const FIRST_FINISHED = "2026-09-24T06:00:47.000Z";
const SECOND_RECORDED = Date.UTC(2026, 8, 24, 6, 1, 13);
const THIRD_STARTED = "2026-09-24T06:02:30.000Z";

/** Three attempts that each exercise a different branch of the entry layout. */
const ATTEMPTS: WorkflowNodeAttemptFailure[] = [
  {
    nodeRunId: "node-run-1",
    attempt: 1,
    kind: "session",
    errorMessage: "agent session ended with an error",
    sourceChain: [
      "agent node review failed",
      "acp session error",
      "connection reset by peer",
    ],
    recordedAt: Date.UTC(2026, 8, 24, 6, 0, 47),
    startedAt: FIRST_STARTED,
    finishedAt: FIRST_FINISHED,
    sessionId: "session-1",
    iteration: 1,
    replacedBy: "automatic_retry",
  },
  {
    // Never started a session: only the recorded time is known.
    nodeRunId: "node-run-2",
    attempt: 2,
    kind: "mystery_failure",
    errorMessage: "the engine gave up",
    sourceChain: [],
    recordedAt: SECOND_RECORDED,
    replacedBy: "manual_resume",
  },
  {
    nodeRunId: "node-run-3",
    attempt: 3,
    kind: "structured_output",
    errorMessage: "agent node review structured output failed: not json",
    sourceChain: ["not json"],
    recordedAt: Date.UTC(2026, 8, 24, 6, 3, 0),
    startedAt: THIRD_STARTED,
    loopRoundIndex: 1,
    beforeRestart: true,
  },
];

function renderAttempts(
  attempts: readonly WorkflowNodeAttemptFailure[] | undefined,
) {
  return render(
    <AppI18nProvider>
      <RunActFailedAttempts attempts={attempts} />
    </AppI18nProvider>,
  );
}

function entries(): HTMLElement[] {
  return Array.from(
    document.querySelectorAll<HTMLElement>(
      '[data-slot="failed-attempts"] ol > li[data-attempt]',
    ),
  );
}

function entry(attempt: number): HTMLElement {
  const found = document.querySelector<HTMLElement>(
    `[data-slot="failed-attempts"] li[data-attempt="${attempt}"]`,
  );
  if (found === null) {
    throw new Error(`no entry for attempt ${attempt}`);
  }
  return found;
}

/** Text of the "<label>: <cause>" line, located by its label span. */
function rootCauseLine(item: HTMLElement, label: string): string | null {
  const labelSpan = within(item).queryByText(label);
  return labelSpan?.parentElement?.textContent ?? null;
}

beforeEach(async () => {
  await appI18n.changeLanguage("zh-CN");
});

afterEach(async () => {
  // Unmount first: switching the language re-renders every mounted useTranslation consumer
  // outside act.
  cleanup();
  await appI18n.changeLanguage("zh-CN");
});

describe("RunActFailedAttempts", () => {
  it("renders nothing without attempts", () => {
    const empty = renderAttempts([]);
    expect(empty.container).toBeEmptyDOMElement();
    empty.unmount();

    const missing = renderAttempts(undefined);
    expect(missing.container).toBeEmptyDOMElement();
  });

  it("lists the attempts under the title in the given order", () => {
    renderAttempts(ATTEMPTS);
    const section = document.querySelector('[data-slot="failed-attempts"]');
    expect(section?.tagName).toBe("SECTION");
    expect(section?.querySelector("h4")?.textContent).toBe("之前失败的尝试");
    expect(entries().map((item) => item.dataset.attempt)).toEqual([
      "1",
      "2",
      "3",
    ]);
  });

  it("keeps the caller's order instead of sorting by attempt number", () => {
    renderAttempts([ATTEMPTS[2]!, ATTEMPTS[0]!, ATTEMPTS[1]!]);
    expect(entries().map((item) => item.dataset.attempt)).toEqual([
      "3",
      "1",
      "2",
    ]);
  });

  it("shows the attempt number, translated kind, message and underlying error", () => {
    renderAttempts(ATTEMPTS);
    const first = entry(1);
    expect(within(first).getByText("第 1 次尝试")).toBeInTheDocument();
    expect(within(first).getByText("智能体会话失败")).toBeInTheDocument();
    expect(
      within(first).getByText("agent session ended with an error"),
    ).toBeInTheDocument();
    expect(rootCauseLine(first, "底层原因")).toBe(
      "底层原因: connection reset by peer",
    );

    const third = entry(3);
    expect(within(third).getByText("第 3 次尝试")).toBeInTheDocument();
    expect(within(third).getByText("结构化输出不合格")).toBeInTheDocument();
    expect(
      within(third).getByText(
        "agent node review structured output failed: not json",
      ),
    ).toBeInTheDocument();
  });

  it("falls back to the raw kind when the kind is unknown", () => {
    renderAttempts(ATTEMPTS);
    const second = entry(2);
    expect(within(second).getByText("第 2 次尝试")).toBeInTheDocument();
    expect(within(second).getByText("mystery_failure")).toBeInTheDocument();
    expect(within(second).getByText("the engine gave up")).toBeInTheDocument();
  });

  it("puts a multi-level chain in collapsed details that open on click", async () => {
    const user = userEvent.setup();
    renderAttempts(ATTEMPTS);
    const details = entry(1).querySelector("details");
    expect(details).not.toBeNull();
    expect(details!.open).toBe(false);
    const summary = within(details!).getByText("完整错误链（3 层）");
    expect(summary.tagName).toBe("SUMMARY");
    expect(
      Array.from(details!.querySelectorAll("ol > li")).map(
        (item) => item.textContent,
      ),
    ).toEqual([
      "agent node review failed",
      "acp session error",
      "connection reset by peer",
    ]);

    await user.click(summary);

    expect(details!.open).toBe(true);
  });

  it("omits the underlying error and the chain for an empty chain", () => {
    renderAttempts(ATTEMPTS);
    const second = entry(2);
    expect(within(second).queryByText("底层原因")).not.toBeInTheDocument();
    expect(second.querySelector("details")).toBeNull();
  });

  it("shows the underlying error but no chain for a one-entry chain", () => {
    renderAttempts(ATTEMPTS);
    const third = entry(3);
    expect(rootCauseLine(third, "底层原因")).toBe("底层原因: not json");
    expect(third.querySelector("details")).toBeNull();
  });

  it("shows the round badge from the region iteration or the Loop round", () => {
    renderAttempts(ATTEMPTS);
    expect(within(entry(1)).getByText("第 2 轮")).toBeInTheDocument();
    expect(within(entry(2)).queryByText(/轮$/u)).not.toBeInTheDocument();
    expect(within(entry(3)).getByText("第 2 轮")).toBeInTheDocument();
  });

  it("prefers the region iteration over a Loop round for the badge", () => {
    renderAttempts([{ ...ATTEMPTS[0]!, iteration: 0, loopRoundIndex: 4 }]);
    expect(within(entry(1)).getByText("第 1 轮")).toBeInTheDocument();
    expect(within(entry(1)).queryByText("第 5 轮")).not.toBeInTheDocument();
  });

  it("shows each replacement chip only on the attempt that carries it", () => {
    renderAttempts(ATTEMPTS);
    const chips = ["已自动重试", "已手动续跑", "从头重新运行前"];
    const present = (item: HTMLElement) =>
      chips.filter((chip) => within(item).queryByText(chip) !== null);
    expect(present(entry(1))).toEqual(["已自动重试"]);
    expect(present(entry(2))).toEqual(["已手动续跑"]);
    expect(present(entry(3))).toEqual(["从头重新运行前"]);
  });

  it("says a scheduled retry that never started was not a retry", async () => {
    const scheduled: WorkflowNodeAttemptFailure = {
      ...ATTEMPTS[0],
      replacedBy: "automatic_retry_scheduled",
    };
    renderAttempts([scheduled]);
    const chip = within(entry(1)).getByText("已安排自动重试（未开始）");
    expect(within(entry(1)).queryByText("已自动重试")).not.toBeInTheDocument();
    // Only a retry that ran shares the waiting state's orange.
    expect(chip.className).not.toContain("orange");

    cleanup();
    await appI18n.changeLanguage("en-US");
    renderAttempts([scheduled]);
    expect(
      within(entry(1)).getByText("Automatic retry scheduled (not started)"),
    ).toBeInTheDocument();
  });

  it("labels the time from start to finish, or the recorded time without a start", () => {
    renderAttempts(ATTEMPTS);
    expect(
      within(entry(1)).getByText(
        `${formatRunClock(FIRST_STARTED, "zh-CN")} — ${formatRunClock(FIRST_FINISHED, "zh-CN")}`,
      ),
    ).toBeInTheDocument();
    expect(
      within(entry(2)).getByText(
        formatRunClock(new Date(SECOND_RECORDED).toISOString(), "zh-CN"),
      ),
    ).toBeInTheDocument();
    expect(
      within(entry(3)).getByText(
        `${formatRunClock(THIRD_STARTED, "zh-CN")} — —`,
      ),
    ).toBeInTheDocument();
  });

  it("renders the English title, kind, chips, chain and times", async () => {
    await appI18n.changeLanguage("en-US");
    renderAttempts(ATTEMPTS);

    expect(screen.getByText("Earlier failed attempts")).toBeInTheDocument();
    const first = entry(1);
    expect(within(first).getByText("Attempt 1")).toBeInTheDocument();
    expect(within(first).getByText("Agent session failed")).toBeInTheDocument();
    expect(within(first).getByText("Round 2")).toBeInTheDocument();
    expect(
      within(first).getByText("Retried automatically"),
    ).toBeInTheDocument();
    expect(rootCauseLine(first, "Underlying error")).toBe(
      "Underlying error: connection reset by peer",
    );
    expect(
      within(first).getByText("Full error chain (3 levels)"),
    ).toBeInTheDocument();
    expect(
      within(first).getByText(
        `${formatRunClock(FIRST_STARTED, "en-US")} — ${formatRunClock(FIRST_FINISHED, "en-US")}`,
      ),
    ).toBeInTheDocument();

    const second = entry(2);
    expect(within(second).getByText("Attempt 2")).toBeInTheDocument();
    expect(within(second).getByText("Resumed manually")).toBeInTheDocument();
    expect(
      within(second).getByText(
        formatRunClock(new Date(SECOND_RECORDED).toISOString(), "en-US"),
      ),
    ).toBeInTheDocument();

    const third = entry(3);
    expect(within(third).getByText("Attempt 3")).toBeInTheDocument();
    expect(within(third).getByText("Round 2")).toBeInTheDocument();
    expect(
      within(third).getByText("Before “Run again from start”"),
    ).toBeInTheDocument();
  });
});
