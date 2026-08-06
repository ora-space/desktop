import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { appI18n } from "../../i18n/i18n-instance";
import { WorkspaceFileViewer } from "./workspace-file-viewer";
import { utf8ByteColumnToStringIndex } from "./workspace-file-viewer-utils";

afterEach(() => vi.restoreAllMocks());

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
    expect(screen.getByText("main").closest("[aria-current=location]")).not.toBeNull();
    expect(scrollIntoView).toHaveBeenCalledWith({
      block: "center",
      inline: "nearest",
    });
  });

  it("enables horizontal scrolling for long source lines", async () => {
    const { container } = render(
      <WorkspaceFileViewer
        content={"a".repeat(300)}
        path="README.md"
        target={null}
      />,
    );

    await waitFor(() => expect(container.querySelector(
      '[data-slot="scroll-area"][data-scrollbars="both"]',
    )).not.toBeNull());
  });

  it("switches large files to plain text mode", async () => {
    const { container } = render(
      <WorkspaceFileViewer
        content={"a".repeat(512 * 1024 + 1)}
        path="large.log"
        target={null}
      />,
    );

    await waitFor(() => expect(container.querySelector("[data-large-file-notice]")).not.toBeNull());
  });

  it("selects a line range with a line click followed by shift-click", async () => {
    const { container } = render(
      <WorkspaceFileViewer
        content={"one\ntwo\nthree\nfour"}
        path="README.md"
        target={null}
      />,
    );

    await waitFor(() => expect(screen.getByRole("button", { name: appI18n.t("files.selectLine", { line: 2 }) })).toBeInTheDocument());
    const start = screen.getByRole("button", { name: appI18n.t("files.selectLine", { line: 2 }) });
    const end = screen.getByRole("button", { name: appI18n.t("files.selectLine", { line: 4 }) });
    fireEvent.click(start);
    fireEvent.click(end, { shiftKey: true });

    const rows = container.querySelectorAll("[data-line-number]");
    expect(rows[0]).not.toHaveClass("bg-sky-500/10");
    expect(rows[1]).toHaveClass("bg-sky-500/10");
    expect(rows[2]).toHaveClass("bg-sky-500/10");
    expect(rows[3]).toHaveClass("bg-sky-500/10");
  });

  it("selects a line range with a left-button drag over line numbers", async () => {
    const { container } = render(
      <WorkspaceFileViewer
        content={"one\ntwo\nthree\nfour"}
        path="README.md"
        target={null}
      />,
    );

    await waitFor(() => expect(screen.getByRole("button", { name: appI18n.t("files.selectLine", { line: 2 }) })).toBeInTheDocument());
    const start = screen.getByRole("button", { name: appI18n.t("files.selectLine", { line: 2 }) });
    const end = screen.getByRole("button", { name: appI18n.t("files.selectLine", { line: 4 }) });
    fireEvent.mouseDown(start, { button: 0 });
    fireEvent.mouseEnter(end, { buttons: 1 });
    fireEvent.mouseUp(end, { button: 0 });

    const rows = container.querySelectorAll("[data-line-number]");
    expect(rows[0]).not.toHaveClass("bg-sky-500/10");
    expect(rows[1]).toHaveClass("bg-sky-500/10");
    expect(rows[2]).toHaveClass("bg-sky-500/10");
    expect(rows[3]).toHaveClass("bg-sky-500/10");
  });
});
