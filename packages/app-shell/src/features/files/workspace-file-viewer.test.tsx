import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { appI18n } from "../../i18n/i18n-instance";
import { WorkspaceFileViewer } from "./workspace-file-viewer";
import {
  lineDisplayColumns,
  utf8ByteColumnToStringIndex,
} from "./workspace-file-viewer-utils";

vi.mock("../../state/actions/add-composer-file-selection", () => ({
  addComposerFileSelections: vi.fn(),
}));

const addComposerFileSelections = vi.mocked(
  await import("../../state/actions/add-composer-file-selection"),
).addComposerFileSelections;

afterEach(() => {
  vi.restoreAllMocks();
  addComposerFileSelections.mockClear();
});

/** Builds a file with one stable, unique line per row above the virtualize threshold. */
const linesContent = (count: number): string =>
  Array.from({ length: count }, (_line, index) => `body ${index + 1}`).join(
    "\n",
  );

/**
 * Gives the virtualizer a viewport size in jsdom. jsdom performs no layout,
 * so every element reports a zero offsetHeight and the window would compute
 * as empty; the virtualizer reads offsetWidth/offsetHeight for its rect.
 */
const mockViewportSize = (height: number) => {
  vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockReturnValue(
    height,
  );
  vi.spyOn(HTMLElement.prototype, "offsetWidth", "get").mockReturnValue(800);
};

describe("WorkspaceFileViewer", () => {
  it("converts UTF-8 byte columns before locating a browser string index", () => {
    expect(utf8ByteColumnToStringIndex("α main", 4)).toBe(2);
  });

  it("scrolls to and highlights the exact selected search match", async () => {
    const scrollIntoView = vi.spyOn(Element.prototype, "scrollIntoView");

    render(
      <WorkspaceFileViewer
        content={"first\nα main()\nlast"}
        path="src/main.rs"
        target={{ line: 2, column: 4, matchedText: "main" }}
      />,
    );

    await waitFor(() => expect(screen.getByText("main")).toBeInTheDocument());
    expect(screen.getByText("main").tagName).toBe("MARK");
    expect(
      screen.getByText("main").closest("[aria-current=location]"),
    ).not.toBeNull();
    expect(scrollIntoView).toHaveBeenCalledWith({
      block: "center",
      inline: "nearest",
    });
  });

  it("highlights every line in a cited range and scrolls to the start", async () => {
    const scrollIntoView = vi.spyOn(Element.prototype, "scrollIntoView");

    const { container } = render(
      <WorkspaceFileViewer
        content={"one\ntwo\nthree\nfour"}
        path="README.md"
        target={{ line: 2, column: 1, matchedText: "", endLine: 4 }}
      />,
    );

    await waitFor(() =>
      expect(container.querySelector('[data-line-number="2"]')).not.toBeNull(),
    );
    expect(container.querySelector('[data-line-number="2"]')).toHaveAttribute(
      "aria-current",
      "location",
    );
    expect(
      container.querySelectorAll("[data-cited-range='true']"),
    ).toHaveLength(3);
    expect(
      [...container.querySelectorAll("[data-cited-range='true']")].map((node) =>
        node.getAttribute("data-line-number"),
      ),
    ).toEqual(["2", "3", "4"]);
    expect(
      container.querySelectorAll(
        "[data-cited-range='true'] [data-quote-gutter]",
      ),
    ).toHaveLength(3);
    expect(
      container.querySelector('[data-line-number="1"]'),
    ).not.toHaveAttribute("aria-current");
    expect(scrollIntoView).toHaveBeenCalledWith({
      block: "center",
      inline: "nearest",
    });
  });

  it("keeps a search match when clicking another line", async () => {
    const { container } = render(
      <WorkspaceFileViewer
        content={"first\nα main()\nlast"}
        path="src/main.rs"
        target={{ line: 2, column: 4, matchedText: "main" }}
      />,
    );

    await waitFor(() => expect(screen.getByText("main")).toBeInTheDocument());
    fireEvent.mouseDown(container.querySelector('[data-line-number="1"]')!, {
      button: 0,
    });

    expect(screen.getByText("main").closest("mark")).not.toBeNull();
    expect(container.querySelector('[data-line-number="2"]')).toHaveAttribute(
      "aria-current",
      "location",
    );
    expect(container.querySelector("[data-cited-range='true']")).toBeNull();
  });

  it("does not dismiss a cited range when clicking the gutter + control", async () => {
    const { container } = render(
      <WorkspaceFileViewer
        content={"one\ntwo\nthree\nfour"}
        path="README.md"
        target={{ line: 2, column: 1, matchedText: "", endLine: 4 }}
      />,
    );

    await waitFor(() =>
      expect(
        screen.getByRole("button", {
          name: appI18n.t("files.quoteLineToChat", { line: 1 }),
        }),
      ).toBeInTheDocument(),
    );
    fireEvent.mouseDown(
      screen.getByRole("button", {
        name: appI18n.t("files.quoteLineToChat", { line: 1 }),
      }),
      { button: 0 },
    );

    expect(
      container.querySelectorAll("[data-cited-range='true']"),
    ).toHaveLength(3);
  });

  it("clears the cited-range highlight when clicking another line", async () => {
    const { container } = render(
      <WorkspaceFileViewer
        content={"one\ntwo\nthree\nfour"}
        path="README.md"
        target={{ line: 2, column: 1, matchedText: "", endLine: 4 }}
      />,
    );

    await waitFor(() =>
      expect(
        container.querySelectorAll("[data-cited-range='true']"),
      ).toHaveLength(3),
    );

    fireEvent.mouseDown(container.querySelector('[data-line-number="1"]')!, {
      button: 0,
    });

    expect(
      container.querySelectorAll("[data-cited-range='true']"),
    ).toHaveLength(0);
    expect(
      container.querySelector('[data-line-number="2"]'),
    ).not.toHaveAttribute("aria-current");
  });

  it("keeps the cited-range highlight when clicking inside it", async () => {
    const { container } = render(
      <WorkspaceFileViewer
        content={"one\ntwo\nthree\nfour"}
        path="README.md"
        target={{ line: 2, column: 1, matchedText: "", endLine: 4 }}
      />,
    );

    await waitFor(() =>
      expect(container.querySelector('[data-line-number="3"]')).not.toBeNull(),
    );

    fireEvent.mouseDown(container.querySelector('[data-line-number="3"]')!, {
      button: 0,
    });

    expect(
      container.querySelectorAll("[data-cited-range='true']"),
    ).toHaveLength(3);
  });

  it("restores the cited-range highlight when a new jump target arrives", async () => {
    const { container, rerender } = render(
      <WorkspaceFileViewer
        content={"one\ntwo\nthree\nfour"}
        path="README.md"
        target={{ line: 2, column: 1, matchedText: "", endLine: 4 }}
      />,
    );

    await waitFor(() =>
      expect(
        container.querySelectorAll("[data-cited-range='true']"),
      ).toHaveLength(3),
    );
    fireEvent.mouseDown(container.querySelector('[data-line-number="1"]')!, {
      button: 0,
    });
    expect(
      container.querySelectorAll("[data-cited-range='true']"),
    ).toHaveLength(0);

    rerender(
      <WorkspaceFileViewer
        content={"one\ntwo\nthree\nfour"}
        path="README.md"
        target={{ line: 2, column: 1, matchedText: "", endLine: 4 }}
      />,
    );

    expect(
      container.querySelectorAll("[data-cited-range='true']"),
    ).toHaveLength(3);
  });

  it("enables horizontal scrolling for long source lines", async () => {
    const { container } = render(
      <WorkspaceFileViewer
        content={"a".repeat(300)}
        path="README.md"
        target={null}
      />,
    );

    await waitFor(() =>
      expect(
        container.querySelector(
          '[data-slot="scroll-area"][data-scrollbars="both"]',
        ),
      ).not.toBeNull(),
    );
  });

  it("switches large files to plain text mode", async () => {
    const { container } = render(
      <WorkspaceFileViewer
        content={"a".repeat(512 * 1024 + 1)}
        path="large.log"
        target={null}
      />,
    );

    await waitFor(() =>
      expect(
        container.querySelector("[data-large-file-notice]"),
      ).not.toBeNull(),
    );
  });

  it("pins a clicked line number and extends with shift-click", async () => {
    const { container } = render(
      <WorkspaceFileViewer
        content={"one\ntwo\nthree\nfour"}
        path="README.md"
        target={null}
      />,
    );

    await waitFor(() =>
      expect(
        screen.getByRole("button", {
          name: appI18n.t("files.selectLine", { line: 2 }),
        }),
      ).toBeInTheDocument(),
    );
    const clickNumber = (line: number, shiftKey = false) => {
      const number = screen.getByRole("button", {
        name: appI18n.t("files.selectLine", { line }),
      });
      // Click always trails a mousedown/mouseup pair in real browsers.
      fireEvent.mouseDown(number, { button: 0 });
      fireEvent.mouseUp(number);
      fireEvent.click(number, { shiftKey });
    };
    clickNumber(2);
    clickNumber(4, true);

    const pinned = container.querySelectorAll("[data-quote-pinned]");
    expect(pinned).toHaveLength(3);
    expect(addComposerFileSelections).not.toHaveBeenCalled();
  });

  it("quotes a single line through the gutter + control", async () => {
    render(
      <WorkspaceFileViewer
        content={"one\ntwo\nthree"}
        path="README.md"
        target={null}
      />,
    );

    await waitFor(() =>
      expect(
        screen.getByRole("button", {
          name: appI18n.t("files.quoteLineToChat", { line: 2 }),
        }),
      ).toBeInTheDocument(),
    );
    fireEvent.click(
      screen.getByRole("button", {
        name: appI18n.t("files.quoteLineToChat", { line: 2 }),
      }),
    );

    expect(addComposerFileSelections).toHaveBeenCalledWith([
      { path: "README.md", startLine: 2, endLine: 2 },
    ]);
  });

  it("quotes a dragged line range tracked through code cells via mousemove", async () => {
    const { container } = render(
      <WorkspaceFileViewer
        content={"one\ntwo\nthree\nfour"}
        path="README.md"
        target={null}
      />,
    );

    await waitFor(() =>
      expect(container.querySelector('[data-quote-key="2"]')).not.toBeNull(),
    );
    const gutter = container.querySelector(
      '[data-quote-key="2"] [data-quote-gutter]',
    );
    const endRow = container.querySelector('[data-quote-key="4"]');
    expect(gutter).not.toBeNull();
    expect(endRow).not.toBeNull();

    fireEvent.mouseDown(gutter!, { button: 0 });
    vi.spyOn(document, "elementFromPoint").mockReturnValue(endRow);
    fireEvent.mouseMove(window, { buttons: 1, clientX: 10, clientY: 10 });
    fireEvent.mouseUp(window);

    expect(addComposerFileSelections).toHaveBeenCalledWith([
      {
        path: "README.md",
        startLine: 2,
        endLine: 4,
      },
    ]);
  });

  it("starts a multi-line drag from the line number while the + control is present", async () => {
    const { container } = render(
      <WorkspaceFileViewer
        content={"one\ntwo\nthree\nfour"}
        path="README.md"
        target={null}
      />,
    );

    await waitFor(() =>
      expect(
        screen.getByRole("button", {
          name: appI18n.t("files.quoteLineToChat", { line: 2 }),
        }),
      ).toBeInTheDocument(),
    );
    const lineNumber = screen.getByRole("button", {
      name: appI18n.t("files.selectLine", { line: 2 }),
    });
    const endRow = container.querySelector('[data-quote-key="4"]');
    expect(endRow).not.toBeNull();

    // Drag must start on the number too; hover/visibility of + cannot block it.
    fireEvent.mouseDown(lineNumber, { button: 0 });
    vi.spyOn(document, "elementFromPoint").mockReturnValue(endRow);
    fireEvent.mouseMove(window, { buttons: 1, clientX: 10, clientY: 10 });
    fireEvent.mouseUp(window);

    expect(addComposerFileSelections).toHaveBeenCalledWith([
      {
        path: "README.md",
        startLine: 2,
        endLine: 4,
      },
    ]);
  });

  it("pins the focused line via keyboard instead of quoting", async () => {
    const { container } = render(
      <WorkspaceFileViewer
        content={"one\ntwo"}
        path="README.md"
        target={null}
      />,
    );

    await waitFor(() =>
      expect(
        screen.getByRole("button", {
          name: appI18n.t("files.selectLine", { line: 2 }),
        }),
      ).toBeInTheDocument(),
    );
    fireEvent.keyDown(
      screen.getByRole("button", {
        name: appI18n.t("files.selectLine", { line: 2 }),
      }),
      { key: "Enter" },
    );

    expect(addComposerFileSelections).not.toHaveBeenCalled();
    expect(
      container
        .querySelector('[data-quote-key="2"]')
        ?.getAttribute("data-quote-pinned"),
    ).toBe("true");
  });

  it("quotes the pinned range from the keyboard with Ctrl+Enter", async () => {
    render(
      <WorkspaceFileViewer
        content={"one\ntwo\nthree\nfour"}
        path="README.md"
        target={null}
      />,
    );

    await waitFor(() =>
      expect(
        screen.getByRole("button", {
          name: appI18n.t("files.selectLine", { line: 2 }),
        }),
      ).toBeInTheDocument(),
    );
    const line = (n: number) =>
      screen.getByRole("button", {
        name: appI18n.t("files.selectLine", { line: n }),
      });

    // Pin 2-3 with Enter + shift-Enter, then quote the pinned range.
    fireEvent.keyDown(line(2), { key: "Enter" });
    fireEvent.keyDown(line(3), { key: "Enter", shiftKey: true });
    expect(addComposerFileSelections).not.toHaveBeenCalled();

    fireEvent.keyDown(line(3), { key: "Enter", ctrlKey: true });
    expect(addComposerFileSelections).toHaveBeenCalledWith([
      { path: "README.md", startLine: 2, endLine: 3 },
    ]);
  });

  it("keeps the quote + control as a sibling of the line-number button without an SVG", async () => {
    const { container } = render(
      <WorkspaceFileViewer
        content={"one\ntwo"}
        path="README.md"
        target={null}
      />,
    );

    await waitFor(() =>
      expect(
        screen.getByRole("button", {
          name: appI18n.t("files.selectLine", { line: 1 }),
        }),
      ).toBeInTheDocument(),
    );
    const lineButton = screen.getByRole("button", {
      name: appI18n.t("files.selectLine", { line: 1 }),
    });
    const quoteButton = screen.getByRole("button", {
      name: appI18n.t("files.quoteLineToChat", { line: 1 }),
    });
    expect(lineButton.contains(quoteButton)).toBe(false);
    expect(quoteButton.parentElement).toBe(lineButton.parentElement);
    expect(container.querySelector("[data-quote-button] svg")).toBeNull();
  });

  it("mounts only a window of rows above the virtualize threshold", async () => {
    mockViewportSize(600);
    const { container } = render(
      <WorkspaceFileViewer
        content={linesContent(500)}
        path="big.log"
        target={null}
      />,
    );

    await waitFor(() =>
      expect(container.querySelector('[data-line-number="1"]')).not.toBeNull(),
    );
    // A 600px viewport shows ~30 rows plus overscan — far fewer than the
    // file's 500 lines.
    const mountedRows = container.querySelectorAll("[data-line-number]");
    expect(mountedRows.length).toBeGreaterThan(0);
    expect(mountedRows.length).toBeLessThan(100);
    expect(container.querySelector('[data-line-number="500"]')).toBeNull();
    // 500 lines is well under the defer threshold: no loading overlay.
    expect(container.querySelector("[data-deferred-loading]")).toBeNull();
  });

  it("never falls back to mounting every row when the viewport is unavailable", async () => {
    // Regression guard for the session-switch stall: a windowed file whose
    // viewport cannot be resolved must render the loading state, not all rows.
    vi.spyOn(HTMLPreElement.prototype, "closest").mockReturnValue(null);
    const { container } = render(
      <WorkspaceFileViewer
        content={linesContent(500)}
        path="big.log"
        target={null}
      />,
    );

    // The virtualizer's own mount/observe update lands after this synchronous
    // render; awaiting it inside `act` keeps that update from escaping the test
    // and tripping the clean-stderr gate.
    await waitFor(() =>
      expect(container.querySelector("[data-deferred-loading]")).not.toBeNull(),
    );
    expect(container.querySelectorAll("[data-line-number]")).toHaveLength(0);
  });

  it("shows a loading overlay for oversized files and brings content in after the first paint", async () => {
    mockViewportSize(600);
    const { container } = render(
      <WorkspaceFileViewer
        content={linesContent(70_000)}
        path="big.log"
        target={null}
      />,
    );

    // First commit: overlay only — the split/scan passes have not run.
    expect(container.querySelector("[data-deferred-loading]")).not.toBeNull();
    expect(container.querySelector("[data-line-number]")).toBeNull();

    await waitFor(() =>
      expect(container.querySelector('[data-line-number="1"]')).not.toBeNull(),
    );
    expect(container.querySelector("[data-deferred-loading]")).toBeNull();
  });

  it("does not flash the loading overlay when an already-mounted file refetches", async () => {
    mockViewportSize(600);
    const content = linesContent(70_000);
    const { container, rerender } = render(
      <WorkspaceFileViewer content={content} path="big.log" target={null} />,
    );

    await waitFor(() =>
      expect(container.querySelector('[data-line-number="1"]')).not.toBeNull(),
    );
    rerender(
      <WorkspaceFileViewer
        content={`${content}\nextra tail`}
        path="big.log"
        target={null}
      />,
    );

    expect(container.querySelector("[data-deferred-loading]")).toBeNull();
    expect(container.querySelector('[data-line-number="1"]')).not.toBeNull();
  });

  it("quotes a rendered line while the large file is windowed", async () => {
    mockViewportSize(600);
    render(
      <WorkspaceFileViewer
        content={linesContent(500)}
        path="big.log"
        target={null}
      />,
    );

    await waitFor(() =>
      expect(
        screen.getByRole("button", {
          name: appI18n.t("files.quoteLineToChat", { line: 2 }),
        }),
      ).toBeInTheDocument(),
    );
    fireEvent.click(
      screen.getByRole("button", {
        name: appI18n.t("files.quoteLineToChat", { line: 2 }),
      }),
    );

    expect(addComposerFileSelections).toHaveBeenCalledWith([
      { path: "big.log", startLine: 2, endLine: 2 },
    ]);
  });

  it("keeps the pin wash on rendered rows after a window shift", async () => {
    mockViewportSize(600);
    const { container, rerender } = render(
      <WorkspaceFileViewer
        content={linesContent(500)}
        path="big.log"
        target={null}
      />,
    );

    await waitFor(() =>
      expect(
        screen.getByRole("button", {
          name: appI18n.t("files.selectLine", { line: 2 }),
        }),
      ).toBeInTheDocument(),
    );
    const clickNumber = (line: number, shiftKey = false) => {
      const number = screen.getByRole("button", {
        name: appI18n.t("files.selectLine", { line }),
      });
      fireEvent.mouseDown(number, { button: 0 });
      fireEvent.mouseUp(number);
      fireEvent.click(number, { shiftKey });
    };
    clickNumber(2);
    clickNumber(4, true);
    expect(container.querySelectorAll("[data-quote-pinned]")).toHaveLength(3);

    // The next render (e.g. a scroll-driven window shift) re-applies the pin
    // declaratively instead of relying on the click-time imperative paint.
    rerender(
      <WorkspaceFileViewer
        content={linesContent(500)}
        path="big.log"
        target={null}
      />,
    );
    expect(
      [...container.querySelectorAll("[data-quote-pinned]")].map((node) =>
        node.getAttribute("data-line-number"),
      ),
    ).toEqual(["2", "3", "4"]);
  });
});

describe("lineDisplayColumns", () => {
  it("counts plain characters as one column each", () => {
    expect(lineDisplayColumns("const x = 1;")).toBe(12);
    expect(lineDisplayColumns("")).toBe(0);
  });

  it("advances tabs to the next multiple of eight", () => {
    expect(lineDisplayColumns("\t")).toBe(8);
    expect(lineDisplayColumns("a\tb")).toBe(9);
  });

  it("counts wide characters as two columns", () => {
    expect(lineDisplayColumns("中文")).toBe(4);
  });
});
