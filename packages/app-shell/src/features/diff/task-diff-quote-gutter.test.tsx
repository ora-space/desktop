import { fireEvent, render, waitFor } from "@testing-library/react";
import {
  getChangeKey,
  parseDiff,
  type ChangeData,
  type FileData,
} from "react-diff-view";
import { afterEach, describe, expect, it, vi } from "vitest";
import { appI18n } from "../../i18n/i18n-instance";
import { useTaskDiffQuoteGutter } from "./task-diff-quote-gutter";

vi.mock("../chat/add-composer-file-selection", () => ({
  addComposerFileSelections: vi.fn(),
}));

const addComposerFileSelections = vi.mocked(
  await import("../chat/add-composer-file-selection"),
).addComposerFileSelections;

const PATCH = [
  "diff --git a/src/example.ts b/src/example.ts",
  "index 1111111..2222222 100644",
  "--- a/src/example.ts",
  "+++ b/src/example.ts",
  "@@ -1,3 +1,3 @@",
  " keep",
  "-old line",
  "+new line",
].join("\n");

afterEach(() => {
  vi.restoreAllMocks();
  addComposerFileSelections.mockClear();
});

/** Stands in for react-diff-view's own gutter content: that side's line number, or nothing. */
function defaultNumber(change: ChangeData, side: "old" | "new"): number | null {
  if (change.type === "normal") {
    return side === "old" ? change.oldLineNumber : change.newLineNumber;
  }
  if (change.type === "delete")
    return side === "old" ? change.lineNumber : null;
  return side === "new" ? change.lineNumber : null;
}

/** Renders the hook's gutter output inside a table so row lookups behave like react-diff-view's DOM. */
function DiffQuoteSurface({
  file,
  viewType = "split",
}: {
  file: FileData;
  viewType?: "unified" | "split";
}) {
  const { renderGutter, quoteRootRef } = useTaskDiffQuoteGutter(file, viewType);
  const changes: ChangeData[] = file.hunks.flatMap((hunk) => hunk.changes);
  return (
    <div
      data-quote-root
      ref={(node) => {
        quoteRootRef.current = node;
      }}
    >
      <table>
        <tbody>
          {changes.map((change) => (
            <tr key={getChangeKey(change)}>
              <td>
                {renderGutter({
                  change,
                  side: "old",
                  inHoverState: false,
                  renderDefault: () => defaultNumber(change, "old"),
                  wrapInAnchor: (node) => node,
                })}
              </td>
              <td>
                {renderGutter({
                  change,
                  side: "new",
                  inHoverState: false,
                  renderDefault: () => defaultNumber(change, "new"),
                  wrapInAnchor: (node) => node,
                })}
              </td>
              <td data-code-cell>{change.type}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

describe("useTaskDiffQuoteGutter", () => {
  it("renders side-scoped keys and + controls only on quoteable cells", async () => {
    const file = parseDiff(PATCH)[0]!;
    const { container } = render(<DiffQuoteSurface file={file} />);

    await waitFor(() =>
      expect(
        container.querySelector('[data-quote-key="new:2"]'),
      ).not.toBeNull(),
    );
    // Insert rows quote on the new side; the delete row quotes on the old side.
    expect(container.querySelector('[data-quote-key="new:2"]')).not.toBeNull();
    const plus = container.querySelector(
      '[data-quote-key="new:2"] [data-quote-button]',
    );
    expect(plus).not.toBeNull();
    expect(plus?.getAttribute("aria-label")).toBe(
      appI18n.t("diff.quoteLineToChat", { line: 2 }),
    );
    expect(container.querySelector("[data-quote-button] svg")).toBeNull();
    expect(container.querySelector('[data-quote-key="old:2"]')).not.toBeNull();
    // Normal rows never render an old-side quote gutter (single-number unified).
    expect(container.querySelector('[data-quote-key="old:1"]')).toBeNull();
  });

  it("keeps old-side numbers on split-view context rows", () => {
    const file = parseDiff(PATCH)[0]!;
    const { container } = render(<DiffQuoteSurface file={file} />);

    // The context row's old gutter is not quoteable, but split view still owes
    // it a line number — collapsing context to one number is unified-only.
    const contextRow = container
      .querySelector('[data-quote-key="new:1"]')!
      .closest("tr")!;
    const oldCell = contextRow.querySelector("td")!;
    expect(oldCell.querySelector("[data-quote-key]")).toBeNull();
    expect(oldCell.textContent).toBe("1");
  });

  it("quotes the pinned range from the keyboard with Ctrl+Enter", async () => {
    const file = parseDiff(PATCH)[0]!;
    const { container } = render(<DiffQuoteSurface file={file} />);

    await waitFor(() =>
      expect(
        container.querySelector('[data-quote-key="new:1"] [data-quote-number]'),
      ).not.toBeNull(),
    );
    const number = container.querySelector(
      '[data-quote-key="new:1"] [data-quote-number]',
    )!;

    // Plain Enter only pins; the + is not tabbable, so Ctrl+Enter is the
    // keyboard's only route to the quote action.
    fireEvent.keyDown(number, { key: "Enter" });
    expect(addComposerFileSelections).not.toHaveBeenCalled();

    fireEvent.keyDown(number, { key: "Enter", ctrlKey: true });
    expect(addComposerFileSelections).toHaveBeenCalledWith([
      {
        path: "src/example.ts",
        startLine: 1,
        endLine: 1,
        snippet: " keep",
        origin: "diff",
        diffSide: "new",
      },
    ]);
  });

  it("commits a new-side drag with the new path", async () => {
    const file = parseDiff(PATCH)[0]!;
    const { container } = render(<DiffQuoteSurface file={file} />);

    const start = container.querySelector('[data-quote-key="new:1"]')!;
    const endCodeCell = container
      .querySelector('[data-quote-key="new:2"]')!
      .closest("tr")!
      .querySelector("[data-code-cell]")!;

    fireEvent.mouseDown(start, { button: 0 });
    // Pointer over the code cell must still resolve to that row's new gutter.
    vi.spyOn(document, "elementFromPoint").mockReturnValue(endCodeCell);
    fireEvent.mouseMove(window, { buttons: 1, clientX: 5, clientY: 5 });
    fireEvent.mouseUp(window);

    expect(addComposerFileSelections).toHaveBeenCalledWith([
      {
        path: "src/example.ts",
        startLine: 1,
        endLine: 2,
        snippet: " keep\n+new line",
        origin: "diff",
        diffSide: "new",
      },
    ]);
  });

  it("locks a drag to the side it started on", async () => {
    const file = parseDiff(PATCH)[0]!;
    const { container } = render(<DiffQuoteSurface file={file} />);

    const start = container.querySelector('[data-quote-key="new:1"]')!;
    // Same tr also carries the old-side delete gutter ("old:2").
    const oldGutter = container.querySelector('[data-quote-key="old:2"]')!;

    fireEvent.mouseDown(start, { button: 0 });
    vi.spyOn(document, "elementFromPoint").mockReturnValue(oldGutter);
    fireEvent.mouseMove(window, { buttons: 1, clientX: 5, clientY: 5 });
    fireEvent.mouseUp(window);

    // Crossing to the old column mid-drag must not turn into a quote.
    expect(addComposerFileSelections).not.toHaveBeenCalled();
  });

  it("tints only the dragged side's cells in a split row", () => {
    const file = parseDiff(PATCH)[0]!;
    const { container } = render(<DiffQuoteSurface file={file} />);
    const start = container.querySelector('[data-quote-key="new:2"]')!;
    const row = start.closest("tr")!;
    const oldTd = row.querySelector("td")!;
    const newTd = start.closest("td")!;
    const codeTd = row.querySelector("[data-code-cell]")!;

    fireEvent.mouseDown(start, { button: 0 });

    expect(newTd.getAttribute("data-quote-selected")).toBe("true");
    expect(codeTd.getAttribute("data-quote-selected")).toBe("true");
    expect(oldTd.getAttribute("data-quote-selected")).toBeNull();
    fireEvent.mouseUp(window);
  });

  it("pins a line number on the quoted side only and does not quote", async () => {
    const file = parseDiff(PATCH)[0]!;
    const { container } = render(<DiffQuoteSurface file={file} />);

    await waitFor(() =>
      expect(
        container.querySelector('[data-quote-key="new:1"] [data-quote-number]'),
      ).not.toBeNull(),
    );
    const number = container.querySelector(
      '[data-quote-key="new:1"] [data-quote-number]',
    )!;
    const newTd = number.closest("td")!;
    const oldTd = number.closest("tr")!.querySelector("td")!;

    fireEvent.mouseDown(number, { button: 0 });
    fireEvent.mouseUp(number);
    fireEvent.click(number);

    expect(newTd.getAttribute("data-quote-pinned")).toBe("true");
    expect(oldTd.getAttribute("data-quote-pinned")).toBeNull();
    expect(addComposerFileSelections).not.toHaveBeenCalled();
  });

  it("does not extend a pin across old and new sides", async () => {
    const file = parseDiff(PATCH)[0]!;
    const { container } = render(<DiffQuoteSurface file={file} />);

    await waitFor(() =>
      expect(
        container.querySelector('[data-quote-key="old:2"] [data-quote-number]'),
      ).not.toBeNull(),
    );
    const newNumber = container.querySelector(
      '[data-quote-key="new:1"] [data-quote-number]',
    )!;
    const oldNumber = container.querySelector(
      '[data-quote-key="old:2"] [data-quote-number]',
    )!;

    fireEvent.mouseDown(newNumber, { button: 0 });
    fireEvent.mouseUp(newNumber);
    fireEvent.click(newNumber);
    fireEvent.mouseDown(oldNumber, { button: 0 });
    fireEvent.mouseUp(oldNumber);
    fireEvent.click(oldNumber, { shiftKey: true });

    const newTd = newNumber.closest("td")!;
    const oldTd = oldNumber.closest("td")!;
    expect(oldTd.getAttribute("data-quote-pinned")).toBe("true");
    expect(newTd.getAttribute("data-quote-pinned")).toBeNull();
    expect(addComposerFileSelections).not.toHaveBeenCalled();
  });

  it("hosts unified plus buttons in the left gutter for both deletes and inserts", () => {
    const file = parseDiff(PATCH)[0]!;
    const { container } = render(
      <DiffQuoteSurface file={file} viewType="unified" />,
    );

    const deletePlus = container.querySelector(
      '[data-quote-key="old:2"] [data-quote-button]',
    );
    expect(deletePlus).not.toBeNull();
    expect(deletePlus?.closest("td")).toBe(
      deletePlus?.closest("tr")?.querySelector("td") ?? null,
    );

    const insertHosts = container.querySelectorAll('[data-quote-key="new:2"]');
    expect(insertHosts).toHaveLength(2);
    expect(insertHosts[0]?.querySelector("[data-quote-button]")).not.toBeNull();
    expect(insertHosts[1]?.querySelector("[data-quote-button]")).toBeNull();
    expect(insertHosts[0]?.closest("td")).toBe(
      insertHosts[0]?.closest("tr")?.querySelector("td") ?? null,
    );
  });

  it("lets a unified drag start on a delete and continue onto the insert below", async () => {
    const file = parseDiff(PATCH)[0]!;
    const { container } = render(
      <DiffQuoteSurface file={file} viewType="unified" />,
    );

    const deleteGutter = container.querySelector('[data-quote-key="old:2"]')!;
    const insertCode = container
      .querySelectorAll('[data-quote-key="new:2"]')[0]!
      .closest("tr")!
      .querySelector("[data-code-cell]")!;

    fireEvent.mouseDown(deleteGutter, { button: 0 });
    vi.spyOn(document, "elementFromPoint").mockReturnValue(insertCode);
    fireEvent.mouseMove(window, { buttons: 1, clientX: 5, clientY: 5 });
    fireEvent.mouseUp(window);

    expect(addComposerFileSelections).toHaveBeenCalledWith([
      {
        path: "src/example.ts",
        startLine: 2,
        endLine: 2,
        snippet: "-old line\n+new line",
        origin: "diff",
      },
    ]);
  });
});
