import { createRef } from "react";
import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { AppI18nProvider } from "../../i18n/i18n";
import { TaskChangesNavigationProvider } from "../diff/task-changes-navigation";
import { ComposerEditor, type ComposerEditorHandle } from "./composer-editor";

function renderComposerWithNavigation() {
  const editorRef = createRef<ComposerEditorHandle>();
  const openDiff = vi.fn();
  const openWorkspaceFile = vi.fn();
  const openWorkspaceDirectory = vi.fn();
  render(
    <AppI18nProvider>
      <TaskChangesNavigationProvider
        onOpenDiff={openDiff}
        onOpenWorkspaceFile={openWorkspaceFile}
        onOpenWorkspaceDirectory={openWorkspaceDirectory}
      >
        <ComposerEditor
          ref={editorRef}
          ariaLabel="Message"
          onSubmit={vi.fn()}
        />
      </TaskChangesNavigationProvider>
    </AppI18nProvider>,
  );
  return { editorRef, openDiff, openWorkspaceFile, openWorkspaceDirectory };
}

describe("ComposerFileChipView navigation", () => {
  it("jumps to the reference on a plain click, marking the chip navigable", async () => {
    const user = userEvent.setup();
    const { editorRef, openWorkspaceFile } = renderComposerWithNavigation();
    const textbox = screen.getByRole("textbox", { name: "Message" });

    act(() => {
      editorRef.current?.insertFileChips([
        { path: "src/app.ts", startLine: 4 },
      ]);
    });
    const chip = await waitFor(() => {
      const el = textbox.querySelector("[data-composer-file='src/app.ts']");
      expect(el).not.toBeNull();
      return el!;
    });
    expect(chip).toHaveAttribute("data-navigable", "true");

    await user.click(chip);

    expect(openWorkspaceFile).toHaveBeenCalledWith("src/app.ts", { line: 4 });
  });

  it("does not navigate when the composer has no navigation context", async () => {
    const user = userEvent.setup();
    const editorRef = createRef<ComposerEditorHandle>();
    render(
      <AppI18nProvider>
        <ComposerEditor
          ref={editorRef}
          ariaLabel="Message"
          onSubmit={vi.fn()}
        />
      </AppI18nProvider>,
    );
    const textbox = screen.getByRole("textbox", { name: "Message" });

    act(() => {
      editorRef.current?.insertFileChips([{ path: "src/app.ts" }]);
    });
    const chip = await waitFor(() => {
      const el = textbox.querySelector("[data-composer-file='src/app.ts']");
      expect(el).not.toBeNull();
      return el!;
    });
    expect(chip).not.toHaveAttribute("data-navigable");

    // Should not throw despite no navigation target.
    await user.click(chip);
  });

  it("shows a diff badge only on diff-origin chips in the composer", async () => {
    const { editorRef } = renderComposerWithNavigation();
    const textbox = screen.getByRole("textbox", { name: "Message" });

    act(() => {
      editorRef.current?.insertFileChips([
        { path: "src/app.ts", startLine: 4, origin: "diff" },
      ]);
    });
    const diffChip = await waitFor(() => {
      const el = textbox.querySelector("[data-composer-file='src/app.ts']");
      expect(el).not.toBeNull();
      return el!;
    });
    expect(
      diffChip.querySelector(".composer-file-ref-diff-icon"),
    ).not.toBeNull();

    act(() => {
      editorRef.current?.insertFileChips([
        { path: "src/lib.ts", startLine: 1 },
      ]);
    });
    const plainChip = await waitFor(() => {
      const el = textbox.querySelector("[data-composer-file='src/lib.ts']");
      expect(el).not.toBeNull();
      return el!;
    });
    expect(plainChip.querySelector(".composer-file-ref-diff-icon")).toBeNull();
  });

  it("still selects (not navigates) on Ctrl+click, preserving delete/drag behaviour", async () => {
    const { editorRef, openWorkspaceFile } = renderComposerWithNavigation();
    const textbox = screen.getByRole("textbox", { name: "Message" });

    act(() => {
      editorRef.current?.insertFileChips([
        { path: "src/app.ts", startLine: 4 },
      ]);
    });
    const chip = await waitFor(() => {
      const el = textbox.querySelector("[data-composer-file='src/app.ts']");
      expect(el).not.toBeNull();
      return el!;
    });

    fireEvent.mouseDown(chip, { button: 0, ctrlKey: true, detail: 1 });
    fireEvent.click(chip, { button: 0, ctrlKey: true, detail: 1 });

    expect(openWorkspaceFile).not.toHaveBeenCalled();
    expect(chip.closest("[data-chip-selected]")).not.toBeNull();
    // A NodeSelection would have hidden the caret via this class.
    expect(textbox.classList.contains("ProseMirror-hideselection")).toBe(false);
  });

  it("still selects (not navigates) on double click", async () => {
    const { editorRef, openWorkspaceFile } = renderComposerWithNavigation();
    const textbox = screen.getByRole("textbox", { name: "Message" });

    act(() => {
      editorRef.current?.insertFileChips([
        { path: "src/app.ts", startLine: 4 },
      ]);
    });
    const chip = await waitFor(() => {
      const el = textbox.querySelector("[data-composer-file='src/app.ts']");
      expect(el).not.toBeNull();
      return el!;
    });

    fireEvent.mouseDown(chip, { button: 0, detail: 2 });
    fireEvent.dblClick(chip, { button: 0, detail: 2 });

    expect(openWorkspaceFile).not.toHaveBeenCalled();
    expect(chip.closest("[data-chip-selected]")).not.toBeNull();
    expect(textbox.classList.contains("ProseMirror-hideselection")).toBe(false);
  });

  it("still navigates on the single click that follows a double click", async () => {
    const { editorRef, openWorkspaceFile } = renderComposerWithNavigation();
    const textbox = screen.getByRole("textbox", { name: "Message" });

    act(() => {
      editorRef.current?.insertFileChips([
        { path: "src/app.ts", startLine: 4 },
      ]);
    });
    const chip = await waitFor(() => {
      const el = textbox.querySelector("[data-composer-file='src/app.ts']");
      expect(el).not.toBeNull();
      return el!;
    });

    // Full double-click sequence: the browser fires the detail=1 click, then
    // the detail=2 press/click pair, and only then `dblclick`.
    fireEvent.mouseDown(chip, { button: 0, detail: 1 });
    fireEvent.click(chip, { button: 0, detail: 1 });
    fireEvent.mouseDown(chip, { button: 0, detail: 2 });
    fireEvent.click(chip, { button: 0, detail: 2 });
    fireEvent.dblClick(chip, { button: 0, detail: 2 });
    // The node-selection dispatch from `dblClick` re-renders the ProseMirror
    // node view on a microtask; flush it before the next click so that
    // update lands inside `act` instead of leaking into the assertion below.
    await act(async () => {
      await Promise.resolve();
    });
    openWorkspaceFile.mockClear();

    // A later plain click must not be eaten by a flag stranded by `dblclick`.
    fireEvent.mouseDown(chip, { button: 0, detail: 1 });
    fireEvent.click(chip, { button: 0, detail: 1 });

    expect(openWorkspaceFile).toHaveBeenCalledWith("src/app.ts", { line: 4 });
  });

  it("jumps a directory mention to the folder, not a file open", async () => {
    const user = userEvent.setup();
    const { editorRef, openWorkspaceDirectory, openWorkspaceFile } =
      renderComposerWithNavigation();
    const textbox = screen.getByRole("textbox", { name: "Message" });

    act(() => {
      editorRef.current?.insertFileChips([
        { path: "src/features", kind: "directory" },
      ]);
    });
    const chip = await waitFor(() => {
      const el = textbox.querySelector("[data-composer-file='src/features']");
      expect(el).not.toBeNull();
      return el!;
    });

    await user.click(chip);

    expect(openWorkspaceDirectory).toHaveBeenCalledWith("src/features");
    expect(openWorkspaceFile).not.toHaveBeenCalled();
  });
});
