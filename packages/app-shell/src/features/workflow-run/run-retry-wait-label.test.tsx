import { act, cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkflowNodeRetryWait } from "@ora/workflow-runtime";
import { AppI18nProvider } from "../../i18n/i18n";
import { appI18n } from "../../i18n/i18n-instance";
import { RunRetryWaitLabel } from "./run-retry-wait-label";

const NOW = Date.UTC(2026, 8, 24, 6, 0, 0);

const WAIT: WorkflowNodeRetryWait = {
  attempt: 2,
  maxAttempt: 4,
  retry: 1,
  maxRetries: 3,
  delayMs: 3_000,
  scheduledAt: NOW,
  dueAt: NOW + 3_000,
};

function renderLabel(
  wait: WorkflowNodeRetryWait,
  variant?: "full" | "compact",
  className?: string,
) {
  const view = render(
    <AppI18nProvider>
      <RunRetryWaitLabel wait={wait} variant={variant} className={className} />
    </AppI18nProvider>,
  );
  return {
    ...view,
    label: () => view.container.querySelector("[data-retry-wait]"),
    rerenderWith: (next: WorkflowNodeRetryWait) =>
      view.rerender(
        <AppI18nProvider>
          <RunRetryWaitLabel
            wait={next}
            variant={variant}
            className={className}
          />
        </AppI18nProvider>,
      ),
  };
}

/**
 * Advances the clock one second per `act`, so React re-renders and re-arms the countdown
 * between ticks the way it does in the browser; one long `act` would fire only the first timer.
 */
function tickSeconds(count: number) {
  for (let second = 0; second < count; second += 1) {
    act(() => {
      vi.advanceTimersByTime(1_000);
    });
  }
}

beforeEach(async () => {
  await appI18n.changeLanguage("zh-CN");
  vi.useFakeTimers();
  vi.setSystemTime(NOW);
});

afterEach(async () => {
  cleanup();
  vi.useRealTimers();
  await appI18n.changeLanguage("zh-CN");
});

describe("RunRetryWaitLabel", () => {
  it("ticks the full label down to the starting text", () => {
    const { label } = renderLabel(WAIT);
    expect(label()?.textContent).toBe("等待重试（第 2/4 次），3 秒后开始");

    tickSeconds(1);
    expect(label()?.textContent).toBe("等待重试（第 2/4 次），2 秒后开始");

    tickSeconds(1);
    expect(label()?.textContent).toBe("等待重试（第 2/4 次），1 秒后开始");

    tickSeconds(1);
    expect(label()?.textContent).toBe("等待重试（第 2/4 次），即将开始");
    expect(vi.getTimerCount()).toBe(0);
  });

  it("ticks the compact label down to the starting text", () => {
    const { label } = renderLabel(WAIT, "compact");
    expect(label()?.textContent).toBe("3 秒后开始");

    tickSeconds(1);
    expect(label()?.textContent).toBe("2 秒后开始");

    tickSeconds(2);
    expect(label()?.textContent).toBe("即将开始");
  });

  it("renders the English full and compact strings", async () => {
    await appI18n.changeLanguage("en-US");
    const full = renderLabel(WAIT);
    expect(full.label()?.textContent).toBe(
      "Waiting to retry (attempt 2/4), starts in 3s",
    );
    tickSeconds(3);
    expect(full.label()?.textContent).toBe(
      "Waiting to retry (attempt 2/4), starting…",
    );
    full.unmount();

    const compact = renderLabel({ ...WAIT, dueAt: NOW + 6_000 }, "compact");
    expect(compact.label()?.textContent).toBe("starts in 3s");
    tickSeconds(3);
    expect(compact.label()?.textContent).toBe("Starting…");
  });

  it("marks the span with its variant, keeps it silent to screen readers and passes the class", () => {
    const full = renderLabel(WAIT, undefined, "text-orange-700");
    const fullSpan = full.label();
    expect(fullSpan?.tagName).toBe("SPAN");
    expect(fullSpan?.getAttribute("data-retry-wait")).toBe("full");
    expect(fullSpan?.getAttribute("aria-live")).toBe("off");
    expect(fullSpan?.className).toBe("text-orange-700");
    full.unmount();

    const compact = renderLabel(WAIT, "compact");
    expect(compact.label()?.getAttribute("data-retry-wait")).toBe("compact");
    expect(compact.label()?.getAttribute("aria-live")).toBe("off");
  });

  it("restarts from the new remaining time when a new wait replaces the old one", () => {
    const { label, rerenderWith } = renderLabel(WAIT);
    tickSeconds(3);
    expect(label()?.textContent).toBe("等待重试（第 2/4 次），即将开始");

    // No timer runs at zero, so the old countdown's clock reading is now 2 s stale.
    tickSeconds(2);
    rerenderWith({
      ...WAIT,
      attempt: 3,
      retry: 2,
      scheduledAt: NOW + 5_000,
      dueAt: NOW + 8_000,
    });
    expect(label()?.textContent).toBe("等待重试（第 3/4 次），3 秒后开始");

    tickSeconds(1);
    expect(label()?.textContent).toBe("等待重试（第 3/4 次），2 秒后开始");

    tickSeconds(2);
    expect(label()?.textContent).toBe("等待重试（第 3/4 次），即将开始");
  });
});
