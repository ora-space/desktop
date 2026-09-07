import { createElement, type ReactNode } from "react";
import { fireEvent, render, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { parseDiff } from "react-diff-view";
import { AppI18nProvider } from "../../i18n/i18n";
import { TaskDiffFocusBody } from "./task-diff-focus-view";

/** Gives the virtualizer a viewport size in jsdom (no layout is performed). */
const mockViewportSize = (height: number) => {
  vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockReturnValue(
    height,
  );
  vi.spyOn(HTMLElement.prototype, "offsetWidth", "get").mockReturnValue(800);
};

afterEach(() => vi.restoreAllMocks());

function bigPatch(): string {
  const body = Array.from({ length: 900 }, (_l, i) => `+line ${i + 1}`);
  return [
    "diff --git a/src/big.ts b/src/big.ts",
    "new file mode 100644",
    "index 0000000..1111111",
    "--- /dev/null",
    "+++ b/src/big.ts",
    `@@ -0,0 +1,900 @@`,
    ...body,
    "",
  ].join("\n");
}

/** A large modify patch: deleted + inserted lines in one big hunk. */
function modifyPatch(path: string, pairs: number): string {
  const lines: string[] = [];
  for (let i = 0; i < pairs; i += 1) {
    lines.push(`-old line ${i}`);
    lines.push(`+new line ${i}`);
  }
  return [
    `diff --git a/${path} b/${path}`,
    "index 1111111..2222222 100644",
    "--- a/${path}",
    "+++ b/${path}",
    `@@ -1,${pairs} +1,${pairs} @@`,
    ...lines,
    "",
  ].join("\n");
}

describe("TaskDiffFocusBody windowed", () => {
  it("windows a huge file to a bounded set of rows", async () => {
    mockViewportSize(600);
    const file = parseDiff(bigPatch())[0]!;
    const wrapper = ({ children }: { children: ReactNode }) =>
      createElement(
        AppI18nProvider,
        null,
        createElement("div", { style: { height: "600px" } }, children),
      );
    const { container } = render(
      <TaskDiffFocusBody file={file} viewType="unified" fileFlash={null} />,
      { wrapper },
    );

    await waitFor(() =>
      expect(container.querySelector(".diff-line")).not.toBeNull(),
    );
    // The file (900 changes → ~15 chunks) is chunk-windowed: only a few of the
    // native chunk tables are mounted at once, not all of them.
    const mountedTables = container.querySelectorAll("table.diff").length;
    expect(mountedTables).toBeGreaterThan(0);
    expect(mountedTables).toBeLessThan(6);
    // Style parity: insert tint classes survive on the native chunk rows.
    expect(container.querySelector(".diff-code-insert")).not.toBeNull();
    expect(container.querySelector(".diff-gutter-insert")).not.toBeNull();
  });

  it("keeps delete/insert tint classes for a modify file", async () => {
    mockViewportSize(600);
    const file = parseDiff(modifyPatch("src/big.ts", 500))[0]!;
    const wrapper = ({ children }: { children: ReactNode }) =>
      createElement(
        AppI18nProvider,
        null,
        createElement("div", { style: { height: "600px" } }, children),
      );
    const { container } = render(
      <TaskDiffFocusBody file={file} viewType="unified" fileFlash={null} />,
      { wrapper },
    );

    await waitFor(() =>
      expect(container.querySelector(".diff-line")).not.toBeNull(),
    );
    await waitFor(() =>
      expect(container.querySelector(".diff-code-delete")).not.toBeNull(),
    );
    expect(container.querySelector(".diff-gutter-delete")).not.toBeNull();
    expect(container.querySelector(".diff-code-insert")).not.toBeNull();
  });

  it("pairs old/new cells correctly in split view", async () => {
    mockViewportSize(600);
    const file = parseDiff(modifyPatch("src/big.ts", 500))[0]!;
    const wrapper = ({ children }: { children: ReactNode }) =>
      createElement(
        AppI18nProvider,
        null,
        createElement("div", { style: { height: "600px" } }, children),
      );
    const { container } = render(
      <TaskDiffFocusBody file={file} viewType="split" fileFlash={null} />,
      { wrapper },
    );

    await waitFor(() =>
      expect(container.querySelector(".diff-line")).not.toBeNull(),
    );
    await waitFor(() =>
      expect(container.querySelector(".diff-line-compare")).not.toBeNull(),
    );
    expect(container.querySelector(".diff-code-delete")).not.toBeNull();
    expect(container.querySelector(".diff-code-insert")).not.toBeNull();
  });

  it("pins a quoted line when a gutter number is clicked", async () => {
    mockViewportSize(600);
    const file = parseDiff(bigPatch())[0]!;
    const wrapper = ({ children }: { children: ReactNode }) =>
      createElement(
        AppI18nProvider,
        null,
        createElement("div", { style: { height: "600px" } }, children),
      );
    const { container } = render(
      <TaskDiffFocusBody file={file} viewType="unified" fileFlash={null} />,
      { wrapper },
    );

    await waitFor(() =>
      expect(
        container.querySelector(".ora-diff-quote-number[role='button']"),
      ).not.toBeNull(),
    );
    const number = container.querySelector<HTMLElement>(
      ".ora-diff-quote-number[role='button']",
    )!;
    fireEvent.click(number);
    expect(container.querySelector("[data-quote-pinned]")).not.toBeNull();
  });

  it("locates a cited line across a windowed file", async () => {
    mockViewportSize(600);
    const file = parseDiff(bigPatch())[0]!;
    const wrapper = ({ children }: { children: ReactNode }) =>
      createElement(
        AppI18nProvider,
        null,
        createElement("div", { style: { height: "600px" } }, children),
      );
    const { container } = render(
      <TaskDiffFocusBody
        file={file}
        viewType="unified"
        fileFlash={null}
        targetLine={850}
      />,
      { wrapper },
    );

    // The cited line (850) is far down the file; the windowed body must still
    // highlight it even though it lives in a later chunk.
    await waitFor(
      () =>
        expect(container.querySelector(".diff-code-selected")).not.toBeNull(),
      { timeout: 4000 },
    );
  });
});
