import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { AppI18nProvider } from "../../i18n/i18n";
import { appI18n } from "../../i18n/i18n-instance";
import { RunStatusBadge, RunStatusMark } from "./run-status-mark";

beforeEach(async () => {
  await appI18n.changeLanguage("zh-CN");
});

afterEach(async () => {
  // Unmount first: switching the language re-renders every mounted useTranslation consumer
  // outside act.
  cleanup();
  await appI18n.changeLanguage("zh-CN");
});

/** Every class used on the rendered subtree, so spin classes on nested glyphs are caught too. */
function classesIn(root: Element): string[] {
  return [root, ...Array.from(root.querySelectorAll("*"))].flatMap((node) =>
    Array.from(node.classList),
  );
}

describe("RunStatusMark retry_waiting", () => {
  it.each([{ live: false }, { live: true }])(
    "renders a still orange retry glyph when not quiet (live: $live)",
    ({ live }) => {
      const { container } = render(
        <RunStatusMark status="retry_waiting" live={live} />,
      );
      const mark = container.querySelector<HTMLElement>(
        '[data-status-mark="retry_waiting"]',
      );
      expect(mark).not.toBeNull();
      expect(mark!.classList.contains("bg-orange-500")).toBe(true);
      expect(mark!.getAttribute("aria-hidden")).toBe("true");
      const glyph = mark!.querySelector("svg");
      expect(glyph?.classList.contains("tabler-icon-refresh")).toBe(true);
      expect(classesIn(mark!).filter((name) => name.includes("spin"))).toEqual(
        [],
      );
    },
  );

  it("spins only for a live running node, not for a live waiting one", () => {
    const running = render(<RunStatusMark status="running" live />);
    expect(
      classesIn(running.container).some((name) =>
        name.includes("animate-spin"),
      ),
    ).toBe(true);
    running.unmount();

    const waiting = render(<RunStatusMark status="retry_waiting" live />);
    expect(
      classesIn(waiting.container).some((name) =>
        name.includes("animate-spin"),
      ),
    ).toBe(false);
  });

  it.each([{ live: false }, { live: true }])(
    "renders an orange dot when quiet (live: $live)",
    ({ live }) => {
      const { container } = render(
        <RunStatusMark status="retry_waiting" quiet live={live} />,
      );
      const dot = container.firstElementChild;
      expect(dot?.tagName).toBe("SPAN");
      expect(dot?.classList.contains("bg-orange-500")).toBe(true);
      expect(dot?.classList.contains("size-1.5")).toBe(true);
      expect(dot?.children).toHaveLength(0);
      expect(container.querySelector("[data-status-mark]")).toBeNull();
    },
  );
});

describe("RunStatusBadge retry_waiting", () => {
  it.each([
    { locale: "zh-CN" as const, label: "等待重试" },
    { locale: "en-US" as const, label: "Waiting to retry" },
  ])("labels the badge in $locale", async ({ locale, label }) => {
    await appI18n.changeLanguage(locale);
    render(
      <AppI18nProvider>
        <RunStatusBadge status="retry_waiting" />
      </AppI18nProvider>,
    );
    const badge = screen.getByText(label);
    expect(badge.textContent).toBe(label);
    for (const name of [
      "border-orange-500/30",
      "bg-orange-500/10",
      "text-orange-800",
    ]) {
      expect(badge.classList.contains(name)).toBe(true);
    }
    expect(
      badge.querySelector('[data-status-mark="retry_waiting"]'),
    ).not.toBeNull();
  });

  it("uses the orange dot in a quiet badge", () => {
    render(
      <AppI18nProvider>
        <RunStatusBadge status="retry_waiting" quiet />
      </AppI18nProvider>,
    );
    const badge = screen.getByText("等待重试");
    expect(badge.querySelector("[data-status-mark]")).toBeNull();
    expect(badge.querySelector(".bg-orange-500.size-1\\.5")).not.toBeNull();
  });
});
