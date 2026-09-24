import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkflowNodeRetryWait } from "@ora/workflow-runtime";
import { appI18n } from "../../i18n/i18n-instance";
import {
  retryCountdownSeconds,
  retryWaitAttemptText,
  retryWaitText,
  useRetryCountdown,
} from "./retry-countdown";

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

describe("retryCountdownSeconds", () => {
  it.each([
    { remaining: 3_000, seconds: 3 },
    { remaining: 2_001, seconds: 3 },
    { remaining: 2_000, seconds: 2 },
    { remaining: 1_999, seconds: 2 },
    { remaining: 1_000, seconds: 1 },
    { remaining: 1, seconds: 1 },
    { remaining: 0, seconds: 0 },
    { remaining: -1, seconds: 0 },
    { remaining: -5_000, seconds: 0 },
  ])("rounds $remaining ms left up to $seconds s", ({ remaining, seconds }) => {
    expect(retryCountdownSeconds(NOW + remaining, NOW)).toBe(seconds);
  });
});

describe("useRetryCountdown", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(NOW);
  });

  afterEach(() => {
    // Unmount while the fake clock is still installed so pending timers are cleared on it.
    cleanup();
    vi.useRealTimers();
  });

  function renderCountdown(dueAt: number) {
    return renderHook(({ due }) => useRetryCountdown(due), {
      initialProps: { due: dueAt },
    });
  }

  it("drops one second per tick and arms nothing once it reaches zero", () => {
    const { result } = renderCountdown(NOW + 3_000);
    expect(result.current).toBe(3);
    expect(vi.getTimerCount()).toBe(1);

    const seen: number[] = [result.current];
    for (let tick = 0; tick < 3; tick += 1) {
      act(() => {
        vi.advanceTimersByTime(1_000);
      });
      seen.push(result.current);
    }

    expect(seen).toEqual([3, 2, 1, 0]);
    expect(vi.getTimerCount()).toBe(0);

    act(() => {
      vi.advanceTimersByTime(5_000);
    });
    expect(result.current).toBe(0);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("drops at the sub-second boundary so the display never lags the deadline", () => {
    const { result } = renderCountdown(NOW + 2_500);
    expect(result.current).toBe(3);

    act(() => {
      vi.advanceTimersByTime(499);
    });
    expect(result.current).toBe(3);

    act(() => {
      vi.advanceTimersByTime(1);
    });
    expect(result.current).toBe(2);

    act(() => {
      vi.advanceTimersByTime(999);
    });
    expect(result.current).toBe(2);

    act(() => {
      vi.advanceTimersByTime(1);
    });
    expect(result.current).toBe(1);

    act(() => {
      vi.advanceTimersByTime(1_000);
    });
    expect(result.current).toBe(0);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("clears its timer when unmounted mid-countdown", () => {
    const { result, unmount } = renderCountdown(NOW + 3_000);
    act(() => {
      vi.advanceTimersByTime(1_000);
    });
    expect(result.current).toBe(2);
    expect(vi.getTimerCount()).toBe(1);

    unmount();

    expect(vi.getTimerCount()).toBe(0);
  });

  it("renders zero and arms nothing for a deadline already in the past", () => {
    const { result } = renderCountdown(NOW - 1_500);
    expect(result.current).toBe(0);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("renders zero and arms nothing exactly at the deadline", () => {
    const { result } = renderCountdown(NOW);
    expect(result.current).toBe(0);
    expect(vi.getTimerCount()).toBe(0);
  });
});

describe("retryWaitText", () => {
  it.each([
    {
      locale: "zh-CN" as const,
      full: "等待重试（第 2/4 次），3 秒后开始",
      fullStarting: "等待重试（第 2/4 次），即将开始",
      compact: "3 秒后开始",
      compactStarting: "即将开始",
    },
    {
      locale: "en-US" as const,
      full: "Waiting to retry (attempt 2/4), starts in 3s",
      fullStarting: "Waiting to retry (attempt 2/4), starting…",
      compact: "starts in 3s",
      compactStarting: "Starting…",
    },
  ])(
    "renders the $locale full and compact forms",
    ({ locale, full, fullStarting, compact, compactStarting }) => {
      const t = appI18n.getFixedT(locale);
      expect(retryWaitText(t, WAIT, 3, "full")).toBe(full);
      expect(retryWaitText(t, WAIT, 0, "full")).toBe(fullStarting);
      expect(retryWaitText(t, WAIT, 3, "compact")).toBe(compact);
      expect(retryWaitText(t, WAIT, 0, "compact")).toBe(compactStarting);
    },
  );

  it("uses the wait's own attempt numbers and the given seconds", () => {
    const t = appI18n.getFixedT("zh-CN");
    const later: WorkflowNodeRetryWait = {
      ...WAIT,
      attempt: 5,
      maxAttempt: 7,
    };
    expect(retryWaitText(t, later, 42, "full")).toBe(
      "等待重试（第 5/7 次），42 秒后开始",
    );
    expect(retryWaitText(t, later, 1, "compact")).toBe("1 秒后开始");
  });
});

describe("retryWaitAttemptText", () => {
  it.each([
    { locale: "zh-CN" as const, text: "等待重试（第 2/4 次）" },
    { locale: "en-US" as const, text: "Waiting to retry (attempt 2/4)" },
  ])("renders the static $locale label", ({ locale, text }) => {
    expect(retryWaitAttemptText(appI18n.getFixedT(locale), WAIT)).toBe(text);
  });
});
