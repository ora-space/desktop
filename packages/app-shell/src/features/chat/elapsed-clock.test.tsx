import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useElapsedDuration } from "./elapsed-clock";

function ElapsedLabel({
  label,
  startedAt,
  durationMs,
}: {
  label: string;
  startedAt?: number;
  durationMs?: number;
}) {
  const elapsed = useElapsedDuration(startedAt, durationMs);
  return <span>{`${label}:${elapsed ?? "missing"}`}</span>;
}

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe("elapsed clock", () => {
  it("shares one interval, pauses while hidden, and catches up when visible", () => {
    vi.useFakeTimers();
    vi.setSystemTime(10_000);
    let visibilityState: DocumentVisibilityState = "visible";
    vi.spyOn(document, "visibilityState", "get").mockImplementation(
      () => visibilityState,
    );
    const intervalSpy = vi.spyOn(window, "setInterval");

    const view = render(
      <>
        <ElapsedLabel label="a" startedAt={8_000} />
        <ElapsedLabel label="b" startedAt={9_000} />
        <ElapsedLabel label="fixed" durationMs={4_000} />
      </>,
    );

    expect(intervalSpy).toHaveBeenCalledTimes(1);
    expect(screen.getByText("a:2000")).toBeInTheDocument();
    expect(screen.getByText("b:1000")).toBeInTheDocument();
    expect(screen.getByText("fixed:4000")).toBeInTheDocument();

    visibilityState = "hidden";
    act(() => document.dispatchEvent(new Event("visibilitychange")));
    act(() => vi.advanceTimersByTime(5_000));
    expect(screen.getByText("a:2000")).toBeInTheDocument();

    visibilityState = "visible";
    act(() => document.dispatchEvent(new Event("visibilitychange")));
    expect(screen.getByText("a:7000")).toBeInTheDocument();
    expect(intervalSpy).toHaveBeenCalledTimes(2);

    view.unmount();
    expect(vi.getTimerCount()).toBe(0);
  });
});
