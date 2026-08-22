import { fireEvent, render, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  buildQuoteSelections,
  quoteKeyFromPoint,
  quotePaintTargets,
  useQuoteLineSelection,
  type QuoteLineAnchor,
} from "./quote-line-selection";

vi.mock("./chat/add-composer-file-selection", () => ({
  addComposerFileSelections: vi.fn(),
}));

const addComposerFileSelections = vi.mocked(
  await import("./chat/add-composer-file-selection"),
).addComposerFileSelections;

afterEach(() => {
  vi.restoreAllMocks();
  addComposerFileSelections.mockClear();
  document.body.replaceChildren();
});

const FILE_ANCHORS: QuoteLineAnchor[] = [1, 2, 3, 4].map((line) => ({
  key: String(line),
  lineNumber: line,
  snippet: `line-${line}`,
  path: "a.ts",
}));

/** Minimal surface: keyed rows with gutter/number/+ children. */
function QuoteSurface({ anchors }: { anchors: QuoteLineAnchor[] }) {
  const {
    rootRef,
    onGutterMouseDown,
    onPlusMouseDown,
    onPlusClick,
    onNumberClick,
    onNumberKeyDown,
  } = useQuoteLineSelection({ anchors });
  return (
    <div
      data-quote-root
      ref={(node) => {
        rootRef.current = node;
      }}
    >
      {anchors.map((anchor) => (
        <div key={anchor.key} data-quote-key={anchor.key}>
          <span
            data-quote-gutter
            onMouseDown={(event) => onGutterMouseDown(event, anchor.key)}
          >
            <button
              type="button"
              data-quote-button
              onMouseDown={(event) => onPlusMouseDown(event, anchor.key)}
              onClick={(event) => onPlusClick(event, anchor.key)}
            >
              +
            </button>
            <span
              data-quote-number
              onClick={(event) => onNumberClick(event, anchor.key)}
              onKeyDown={(event) => onNumberKeyDown(event, anchor.key)}
            >
              {anchor.lineNumber}
            </span>
          </span>
          <span data-code>{anchor.snippet}</span>
        </div>
      ))}
    </div>
  );
}

describe("buildQuoteSelections", () => {
  it("joins contiguous lines into one run and splits gaps", () => {
    const anchors: QuoteLineAnchor[] = [
      { key: "1", lineNumber: 1, snippet: "one", path: "a.ts" },
      { key: "2", lineNumber: 2, snippet: "two", path: "a.ts" },
      { key: "9", lineNumber: 9, snippet: "nine", path: "a.ts" },
    ];
    expect(buildQuoteSelections(anchors)).toEqual([
      { path: "a.ts", startLine: 1, endLine: 2, snippet: "one\ntwo" },
      { path: "a.ts", startLine: 9, endLine: 9, snippet: "nine" },
    ]);
  });

  it("splits file-style runs when the path changes mid-range", () => {
    const anchors: QuoteLineAnchor[] = [
      { key: "1", lineNumber: 1, snippet: "one", path: "a.ts" },
      { key: "2", lineNumber: 2, snippet: "two", path: "b.ts" },
    ];
    expect(buildQuoteSelections(anchors)).toEqual([
      { path: "a.ts", startLine: 1, endLine: 1, snippet: "one" },
      { path: "b.ts", startLine: 2, endLine: 2, snippet: "two" },
    ]);
  });

  it("keeps one Diff chip when a drag crosses delete, insert, and overlapping lines", () => {
    const anchors: QuoteLineAnchor[] = [
      {
        key: "new:1",
        lineNumber: 1,
        snippet: " keep",
        path: "a.ts",
        origin: "diff",
        diffSide: "new",
      },
      {
        key: "old:2",
        lineNumber: 2,
        snippet: "-gone",
        path: "a.ts",
        origin: "diff",
        diffSide: "old",
      },
      {
        key: "new:2",
        lineNumber: 2,
        snippet: "+added",
        path: "a.ts",
        origin: "diff",
        diffSide: "new",
      },
      {
        key: "new:3",
        lineNumber: 3,
        snippet: " keep-end",
        path: "a.ts",
        origin: "diff",
        diffSide: "new",
      },
    ];
    expect(buildQuoteSelections(anchors)).toEqual([
      {
        path: "a.ts",
        startLine: 1,
        endLine: 3,
        snippet: " keep\n-gone\n+added\n keep-end",
        origin: "diff",
      },
    ]);
  });

  it("keeps one Diff chip across a collapsed hunk gap", () => {
    const anchors: QuoteLineAnchor[] = [
      {
        key: "new:2",
        lineNumber: 2,
        snippet: "+near",
        path: "a.ts",
        origin: "diff",
        diffSide: "new",
      },
      {
        key: "new:40",
        lineNumber: 40,
        snippet: "+far",
        path: "a.ts",
        origin: "diff",
        diffSide: "new",
      },
    ];
    expect(buildQuoteSelections(anchors)).toEqual([
      {
        path: "a.ts",
        startLine: 2,
        endLine: 40,
        snippet: "+near\n+far",
        origin: "diff",
        diffSide: "new",
      },
    ]);
  });

  it("prefers the new-side path when a Diff run spans a rename", () => {
    const anchors: QuoteLineAnchor[] = [
      {
        key: "old:5",
        lineNumber: 5,
        snippet: "-gone",
        path: "old.ts",
        origin: "diff",
        diffSide: "old",
      },
      {
        key: "new:6",
        lineNumber: 6,
        snippet: "+added",
        path: "new.ts",
        origin: "diff",
        diffSide: "new",
      },
    ];
    expect(buildQuoteSelections(anchors)).toEqual([
      {
        path: "new.ts",
        startLine: 5,
        endLine: 6,
        snippet: "-gone\n+added",
        origin: "diff",
      },
    ]);
  });

  it("keeps diff origin and side on a contiguous new-side run", () => {
    const anchors: QuoteLineAnchor[] = [
      {
        key: "new:1",
        lineNumber: 1,
        snippet: " keep",
        path: "a.ts",
        origin: "diff",
        diffSide: "new",
      },
      {
        key: "new:2",
        lineNumber: 2,
        snippet: "+added",
        path: "a.ts",
        origin: "diff",
        diffSide: "new",
      },
    ];
    expect(buildQuoteSelections(anchors)).toEqual([
      {
        path: "a.ts",
        startLine: 1,
        endLine: 2,
        snippet: " keep\n+added",
        origin: "diff",
        diffSide: "new",
      },
    ]);
  });
});

describe("quoteKeyFromPoint", () => {
  function diffRow(): { row: HTMLTableRowElement; code: HTMLTableCellElement } {
    const row = document.createElement("tr");
    const oldGutter = document.createElement("td");
    const oldInner = document.createElement("span");
    oldInner.dataset.quoteKey = "old:5";
    oldInner.dataset.quoteGroup = "old";
    oldGutter.append(oldInner);
    const newGutter = document.createElement("td");
    const newInner = document.createElement("span");
    newInner.dataset.quoteKey = "new:7";
    newInner.dataset.quoteGroup = "new";
    newGutter.append(newInner);
    const code = document.createElement("td");
    code.textContent = "code";
    row.append(oldGutter, newGutter, code);
    return { row, code };
  }

  it("resolves a directly hit keyed element", () => {
    const root = document.createElement("div");
    const row = document.createElement("span");
    row.dataset.quoteKey = "3";
    root.append(row);
    document.body.append(root);
    vi.spyOn(document, "elementFromPoint").mockReturnValue(row);

    expect(quoteKeyFromPoint(0, 0, root, "any")).toBe("3");
  });

  it("falls back from a code cell to the row gutter of the same side", () => {
    const root = document.createElement("div");
    const { row, code } = diffRow();
    root.append(row);
    document.body.append(root);
    vi.spyOn(document, "elementFromPoint").mockReturnValue(code);

    expect(quoteKeyFromPoint(0, 0, root, "new")).toBe("new:7");
    expect(quoteKeyFromPoint(0, 0, root, "old")).toBe("old:5");
  });

  it("ignores a directly hit gutter from the wrong side", () => {
    const root = document.createElement("div");
    const { row } = diffRow();
    root.append(row);
    document.body.append(root);
    const oldInner = row.querySelector<HTMLElement>(
      '[data-quote-key="old:5"]',
    )!;
    vi.spyOn(document, "elementFromPoint").mockReturnValue(oldInner);

    // Dragging the new side over the old gutter must not jump sides.
    expect(quoteKeyFromPoint(0, 0, root, "new")).toBe("new:7");
    expect(quoteKeyFromPoint(0, 0, root, "old")).toBe("old:5");
  });

  it("rejects elements outside the root surface", () => {
    const root = document.createElement("div");
    document.body.append(root);
    const outside = document.createElement("span");
    outside.dataset.quoteKey = "9";
    document.body.append(outside);
    vi.spyOn(document, "elementFromPoint").mockReturnValue(outside);

    expect(quoteKeyFromPoint(0, 0, root, "any")).toBeNull();
  });

  it("accepts either side when the group lock is any", () => {
    const root = document.createElement("div");
    const { row } = diffRow();
    root.append(row);
    document.body.append(root);
    const oldInner = row.querySelector<HTMLElement>(
      '[data-quote-key="old:5"]',
    )!;
    vi.spyOn(document, "elementFromPoint").mockReturnValue(oldInner);

    expect(quoteKeyFromPoint(0, 0, root, "any")).toBe("old:5");
  });
});

describe("quotePaintTargets", () => {
  it("returns the keyed row when there is no table cell", () => {
    const row = document.createElement("span");
    row.dataset.quoteKey = "3";
    expect(quotePaintTargets(row)).toEqual([row]);
  });

  it("paints one split side's gutter and code cells, not the opposite gutter", () => {
    const tr = document.createElement("tr");
    const oldGutter = document.createElement("td");
    oldGutter.className = "diff-gutter";
    const oldInner = document.createElement("span");
    oldInner.dataset.quoteKey = "old:5";
    oldInner.dataset.quoteGroup = "old";
    oldInner.dataset.quoteGutter = "";
    oldGutter.append(oldInner);
    const oldCode = document.createElement("td");
    oldCode.className = "diff-code";
    const newGutter = document.createElement("td");
    newGutter.className = "diff-gutter";
    const newInner = document.createElement("span");
    newInner.dataset.quoteKey = "new:7";
    newInner.dataset.quoteGroup = "new";
    newInner.dataset.quoteGutter = "";
    newGutter.append(newInner);
    const newCode = document.createElement("td");
    newCode.className = "diff-code";
    tr.append(oldGutter, oldCode, newGutter, newCode);

    expect(quotePaintTargets(newInner)).toEqual([newGutter, newCode]);
    expect(quotePaintTargets(oldInner)).toEqual([oldGutter, oldCode]);
  });

  it("walks past the other unified number column to the code cell", () => {
    const tr = document.createElement("tr");
    const oldGutter = document.createElement("td");
    oldGutter.className = "diff-gutter";
    const oldInner = document.createElement("span");
    oldInner.dataset.quoteKey = "new:13";
    oldInner.dataset.quoteGutter = "";
    oldGutter.append(oldInner);
    const newGutter = document.createElement("td");
    newGutter.className = "diff-gutter";
    const newInner = document.createElement("span");
    newInner.dataset.quoteKey = "new:13";
    newInner.dataset.quoteGutter = "";
    newGutter.append(newInner);
    const code = document.createElement("td");
    code.className = "diff-code";
    tr.append(oldGutter, newGutter, code);

    expect(quotePaintTargets(oldInner)).toEqual([oldGutter, newGutter, code]);
  });
});

describe("useQuoteLineSelection", () => {
  it("quotes a single line from the + click", async () => {
    const { container } = render(<QuoteSurface anchors={FILE_ANCHORS} />);
    await waitFor(() =>
      expect(container.querySelector('[data-quote-key="2"]')).not.toBeNull(),
    );
    const plus = container.querySelector(
      '[data-quote-key="2"] [data-quote-button]',
    )!;
    fireEvent.click(plus);

    expect(addComposerFileSelections).toHaveBeenCalledWith([
      { path: "a.ts", startLine: 2, endLine: 2, snippet: "line-2" },
    ]);
  });

  it("drags from the gutter across code cells and commits the range", async () => {
    const { container } = render(<QuoteSurface anchors={FILE_ANCHORS} />);
    const gutter = container.querySelector(
      '[data-quote-key="2"] [data-quote-gutter]',
    )!;
    const endRow = container.querySelector('[data-quote-key="4"]')!;

    fireEvent.mouseDown(gutter, { button: 0 });
    vi.spyOn(document, "elementFromPoint").mockReturnValue(endRow);
    fireEvent.mouseMove(window, { buttons: 1, clientX: 5, clientY: 5 });
    fireEvent.mouseUp(window);

    expect(addComposerFileSelections).toHaveBeenCalledWith([
      {
        path: "a.ts",
        startLine: 2,
        endLine: 4,
        snippet: "line-2\nline-3\nline-4",
      },
    ]);
  });

  it("starts the same drag when pressing the + button directly", async () => {
    const { container } = render(<QuoteSurface anchors={FILE_ANCHORS} />);
    const plus = container.querySelector(
      '[data-quote-key="1"] [data-quote-button]',
    )!;
    const endRow = container.querySelector('[data-quote-key="3"]')!;

    fireEvent.mouseDown(plus, { button: 0 });
    vi.spyOn(document, "elementFromPoint").mockReturnValue(endRow);
    fireEvent.mouseMove(window, { buttons: 1, clientX: 5, clientY: 5 });
    fireEvent.mouseUp(window);

    expect(addComposerFileSelections).toHaveBeenCalledWith([
      {
        path: "a.ts",
        startLine: 1,
        endLine: 3,
        snippet: "line-1\nline-2\nline-3",
      },
    ]);
  });

  it("keeps dragged high-water when returning to the anchor line", async () => {
    const { container } = render(<QuoteSurface anchors={FILE_ANCHORS} />);
    const gutter = container.querySelector(
      '[data-quote-key="2"] [data-quote-gutter]',
    )!;
    const midRow = container.querySelector('[data-quote-key="4"]')!;
    const startRow = container.querySelector('[data-quote-key="2"]')!;

    fireEvent.mouseDown(gutter, { button: 0 });
    const fromPoint = vi.spyOn(document, "elementFromPoint");
    fromPoint.mockReturnValue(midRow);
    fireEvent.mouseMove(window, { buttons: 1, clientX: 5, clientY: 5 });
    fromPoint.mockReturnValue(startRow);
    fireEvent.mouseMove(window, { buttons: 1, clientX: 5, clientY: 6 });
    fireEvent.mouseUp(window);

    expect(addComposerFileSelections).toHaveBeenCalledWith([
      { path: "a.ts", startLine: 2, endLine: 2, snippet: "line-2" },
    ]);
  });

  it("suppresses the click that follows a drag ending on the + button", async () => {
    const { container } = render(<QuoteSurface anchors={FILE_ANCHORS} />);
    const plus = container.querySelector(
      '[data-quote-key="1"] [data-quote-button]',
    )!;
    const endRow = container.querySelector('[data-quote-key="3"]')!;

    fireEvent.mouseDown(plus, { button: 0 });
    vi.spyOn(document, "elementFromPoint").mockReturnValue(endRow);
    fireEvent.mouseMove(window, { buttons: 1, clientX: 5, clientY: 5 });
    fireEvent.mouseUp(plus);
    fireEvent.click(plus);

    // Only the range commit; the trailing click must not re-quote line 1.
    expect(addComposerFileSelections).toHaveBeenCalledTimes(1);
    expect(addComposerFileSelections).toHaveBeenCalledWith([
      {
        path: "a.ts",
        startLine: 1,
        endLine: 3,
        snippet: "line-1\nline-2\nline-3",
      },
    ]);
  });

  it("still quotes on a plain + press without movement", async () => {
    const { container } = render(<QuoteSurface anchors={FILE_ANCHORS} />);
    const plus = container.querySelector(
      '[data-quote-key="3"] [data-quote-button]',
    )!;

    fireEvent.mouseDown(plus, { button: 0 });
    fireEvent.mouseUp(plus);
    fireEvent.click(plus);

    expect(addComposerFileSelections).toHaveBeenCalledWith([
      { path: "a.ts", startLine: 3, endLine: 3, snippet: "line-3" },
    ]);
  });

  it("pins a clicked number and extends the pin with shift-click", async () => {
    const { container } = render(<QuoteSurface anchors={FILE_ANCHORS} />);
    const numberAt = (key: string) =>
      container.querySelector(`[data-quote-key="${key}"] [data-quote-number]`)!;

    // Real browsers always dispatch mousedown before click; the pin must
    // survive the drag state that mousedown sets up.
    fireEvent.mouseDown(numberAt("2"), { button: 0 });
    fireEvent.mouseUp(numberAt("2"));
    fireEvent.click(numberAt("2"));
    fireEvent.mouseDown(numberAt("4"), { button: 0 });
    fireEvent.mouseUp(numberAt("4"));
    fireEvent.click(numberAt("4"), { shiftKey: true });

    const pinned = container.querySelectorAll("[data-quote-pinned]");
    expect(pinned).toHaveLength(3);
    expect(addComposerFileSelections).not.toHaveBeenCalled();
  });

  it("replaces the pin once a real drag travels", async () => {
    const { container } = render(<QuoteSurface anchors={FILE_ANCHORS} />);
    const numberAt = (key: string) =>
      container.querySelector(`[data-quote-key="${key}"] [data-quote-number]`)!;

    fireEvent.mouseDown(numberAt("1"), { button: 0 });
    fireEvent.mouseUp(numberAt("1"));
    fireEvent.click(numberAt("1"));
    expect(container.querySelectorAll("[data-quote-pinned]")).toHaveLength(1);

    const endRow = container.querySelector('[data-quote-key="3"]')!;
    fireEvent.mouseDown(numberAt("2"), { button: 0 });
    vi.spyOn(document, "elementFromPoint").mockReturnValue(endRow);
    fireEvent.mouseMove(window, { buttons: 1, clientX: 5, clientY: 5 });
    fireEvent.mouseUp(window);

    expect(container.querySelectorAll("[data-quote-pinned]")).toHaveLength(0);
    expect(addComposerFileSelections).toHaveBeenCalledWith([
      { path: "a.ts", startLine: 2, endLine: 3, snippet: "line-2\nline-3" },
    ]);
  });

  it("cancels a drag without committing when the button is lost mid-gesture", async () => {
    const { container } = render(<QuoteSurface anchors={FILE_ANCHORS} />);
    const gutter = container.querySelector(
      '[data-quote-key="1"] [data-quote-gutter]',
    )!;

    fireEvent.mouseDown(gutter, { button: 0 });
    // Pointer released outside the window: next move has no buttons pressed.
    fireEvent.mouseMove(window, { buttons: 0, clientX: 5, clientY: 5 });
    fireEvent.mouseUp(window);

    expect(addComposerFileSelections).not.toHaveBeenCalled();
  });

  it("pins on Enter/Space instead of quoting, matching the select-line label", () => {
    const { container } = render(<QuoteSurface anchors={FILE_ANCHORS} />);
    const number = container.querySelector(
      '[data-quote-key="2"] [data-quote-number]',
    )!;

    fireEvent.keyDown(number, { key: "Enter" });
    expect(container.querySelectorAll("[data-quote-pinned]")).toHaveLength(1);
    expect(addComposerFileSelections).not.toHaveBeenCalled();

    fireEvent.keyDown(
      container.querySelector('[data-quote-key="4"] [data-quote-number]')!,
      { key: " ", shiftKey: true },
    );
    expect(container.querySelectorAll("[data-quote-pinned]")).toHaveLength(3);
    expect(addComposerFileSelections).not.toHaveBeenCalled();
  });
});
