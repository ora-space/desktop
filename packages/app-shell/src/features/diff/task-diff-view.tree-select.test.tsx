import { createElement, type ReactNode } from "react";
import { act, fireEvent, render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, describe, expect, it, vi } from "vitest";
import { AppI18nProvider } from "../../i18n/i18n";
import { ContractsClientContext } from "../../contracts-client-context";
import {
  createTestClient,
  type TestHandlers,
} from "../../test/contracts-transport";
import "../../i18n/i18n-instance";
import { TaskDiffView } from "./task-diff-view";
import { DIFF_FILE_SCROLL_INSET_PX } from "./task-diff-scroll";

/** Builds a tiny new-file patch so tests can name the requested path independently of order. */
function addedFilePatch(path: string, body: string): string {
  return [
    `diff --git a/${path} b/${path}`,
    "new file mode 100644",
    "index 0000000..1111111",
    "--- /dev/null",
    `+++ b/${path}`,
    "@@ -0,0 +1,1 @@",
    `+${body}`,
    "",
  ].join("\n");
}

const FIRST_FILE = "docs/specs/alpha.md";
const MIDDLE_FILE = "docs/specs/beta.md";
const LAST_FILE = "docs/specs/gamma.md";

/** Document offsets of the three files; the middle one is the tree-click target. */
const FILE_OFFSETS: Record<string, number> = {
  [FIRST_FILE]: 0,
  [MIDDLE_FILE]: 100,
  [LAST_FILE]: 200,
};
const FILE_HEIGHT = 100;
const VIEWPORT_HEIGHT = 400;

const PATCH = [
  addedFilePatch(FIRST_FILE, "# alpha"),
  addedFilePatch(MIDDLE_FILE, "# beta"),
  addedFilePatch(LAST_FILE, "# gamma"),
].join("");

/** Renders Changes with the mocked three-file patch and no file request. */
function renderTreeDiff() {
  const clientHandlers: TestHandlers = {};
  const client = createTestClient(clientHandlers);
  clientHandlers.getWorkspaceDiff = async () => ({
    baseCommitId: "base",
    headCommitId: "head",
    patch: PATCH,
  });
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
  const wrapper = ({ children }: { children: ReactNode }) =>
    createElement(
      QueryClientProvider,
      { client: queryClient },
      createElement(
        ContractsClientContext.Provider,
        { value: client },
        createElement(AppI18nProvider, null, children),
      ),
    );
  return render(
    <TaskDiffView
      workspaceId="task-1"
      hasBaseline
      viewType="unified"
      fileTreeOpen
      onFileTreeOpenChange={() => undefined}
    />,
    { wrapper },
  );
}

/** Empty box used while the Changes panel has no real layout. */
function emptyRect(): DOMRect {
  return {
    x: 0,
    y: 0,
    top: 0,
    left: 0,
    right: 0,
    bottom: 0,
    width: 0,
    height: 0,
    toJSON() {
      return {};
    },
  };
}

/** Axis-aligned box at `top` with the given size. */
function boxRect(top: number, width: number, height: number): DOMRect {
  return {
    x: 0,
    y: top,
    top,
    left: 0,
    right: width,
    bottom: top + height,
    width,
    height,
    toJSON() {
      return {};
    },
  };
}

const scrollToSpy = vi.fn();

/** Original bounding-box implementation, restored between tests. */
const nativeGetBoundingClientRect = HTMLElement.prototype.getBoundingClientRect;

interface GeometryState {
  /** False until the test lets the scroll viewport report a real height. */
  layoutReady: boolean;
}

/**
 * Stubs the scroll geometry of the Changes viewport and its file sections.
 * With `layoutReady` false nothing can scroll, which keeps a started jump in
 * its retry loop so tests can interact with the run mid-flight.
 */
function installScrollGeometry(state: GeometryState) {
  let scrollPosition = 0;
  Object.defineProperty(HTMLElement.prototype, "scrollTo", {
    configurable: true,
    writable: true,
    value(this: HTMLElement, arg?: ScrollToOptions | number) {
      if (
        this.classList.contains("ora-diff-scroll-region") &&
        typeof arg === "object" &&
        arg !== null
      ) {
        scrollPosition = arg.top ?? 0;
        scrollToSpy(arg);
      }
    },
  });
  Object.defineProperty(HTMLElement.prototype, "scrollTop", {
    configurable: true,
    get() {
      return this.classList?.contains("ora-diff-scroll-region")
        ? scrollPosition
        : 0;
    },
  });
  Object.defineProperty(HTMLElement.prototype, "clientHeight", {
    configurable: true,
    get() {
      if (!this.classList?.contains("ora-diff-scroll-region")) return 0;
      return state.layoutReady ? VIEWPORT_HEIGHT : 0;
    },
  });
  Object.defineProperty(HTMLElement.prototype, "offsetHeight", {
    configurable: true,
    get() {
      if (!state.layoutReady) return 0;
      if (this.getAttribute?.("data-diff-path") in FILE_OFFSETS)
        return FILE_HEIGHT;
      return 0;
    },
  });
  Object.defineProperty(HTMLElement.prototype, "offsetTop", {
    configurable: true,
    get() {
      if (!state.layoutReady) return 0;
      return FILE_OFFSETS[this.getAttribute?.("data-diff-path") ?? ""] ?? 0;
    },
  });
  HTMLElement.prototype.getBoundingClientRect = function () {
    if (!state.layoutReady) return emptyRect();
    if (this.classList.contains("ora-diff-scroll-region")) {
      return boxRect(0, 800, VIEWPORT_HEIGHT);
    }
    const path = this.getAttribute("data-diff-path") ?? "";
    if (path in FILE_OFFSETS) {
      return boxRect(FILE_OFFSETS[path]! - scrollPosition, 800, FILE_HEIGHT);
    }
    return nativeGetBoundingClientRect.call(this);
  };
}

/** Lets the requested number of animation frames elapse inside act, so the
 * scroll spy's rAF-deferred selection updates stay wrapped like real events. */
async function settleFrames(count: number): Promise<void> {
  await act(async () => {
    for (let i = 0; i < count; i++) {
      await new Promise<void>((resolve) => {
        requestAnimationFrame(() => resolve());
      });
    }
  });
}

function scrollRegion(): HTMLElement {
  return document.querySelector<HTMLElement>(".ora-diff-scroll-region")!;
}

describe("TaskDiffView file tree selection", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    HTMLElement.prototype.getBoundingClientRect = nativeGetBoundingClientRect;
    Reflect.deleteProperty(HTMLElement.prototype, "scrollTo");
    Reflect.deleteProperty(HTMLElement.prototype, "clientHeight");
    Reflect.deleteProperty(HTMLElement.prototype, "offsetHeight");
    Reflect.deleteProperty(HTMLElement.prototype, "offsetTop");
    Reflect.deleteProperty(HTMLElement.prototype, "scrollTop");
    scrollToSpy.mockClear();
  });

  it("scrolls a tree-selected file to the viewport top and keeps it selected", async () => {
    // Mount like the other tests with no layout, then expose real geometry
    // before clicking so the jump lands on its first attempt.
    const state: GeometryState = { layoutReady: false };
    installScrollGeometry(state);
    renderTreeDiff();

    const beta = await screen.findByRole("button", { name: "beta.md" });
    state.layoutReady = true;
    await settleFrames(2);
    fireEvent.click(beta);

    expect(beta).toHaveAttribute("aria-current", "page");
    expect(scrollToSpy).toHaveBeenCalledWith({
      top: FILE_OFFSETS[MIDDLE_FILE]! - DIFF_FILE_SCROLL_INSET_PX,
      behavior: "auto",
    });
    await settleFrames(3);
    expect(screen.getByRole("button", { name: "beta.md" })).toHaveAttribute(
      "aria-current",
      "page",
    );
    expect(
      screen.getByRole("button", { name: "gamma.md" }),
    ).not.toHaveAttribute("aria-current");
  });

  it("holds the scroll spy until the jump settles, then follows the user again", async () => {
    const state: GeometryState = { layoutReady: false };
    installScrollGeometry(state);
    renderTreeDiff();

    const beta = await screen.findByRole("button", { name: "beta.md" });
    fireEvent.click(beta);

    // A scroll event while the jump is still settling must not move the
    // selection; the bottom-clamped stub geometry would pick the last file.
    fireEvent.scroll(scrollRegion());
    await settleFrames(2);
    expect(beta).toHaveAttribute("aria-current", "page");

    // The viewport lays out, the jump lands, and the spy takes over again.
    scrollToSpy.mockClear();
    state.layoutReady = true;
    await settleFrames(4);
    expect(scrollToSpy).toHaveBeenCalledWith({
      top: FILE_OFFSETS[MIDDLE_FILE]! - DIFF_FILE_SCROLL_INSET_PX,
      behavior: "auto",
    });
    fireEvent.scroll(scrollRegion());
    await settleFrames(2);
    expect(screen.getByRole("button", { name: "gamma.md" })).toHaveAttribute(
      "aria-current",
      "page",
    );
  });

  it("replaces the in-flight jump when another file is clicked", async () => {
    const state: GeometryState = { layoutReady: false };
    installScrollGeometry(state);
    renderTreeDiff();

    const beta = await screen.findByRole("button", { name: "beta.md" });
    fireEvent.click(beta);
    fireEvent.click(screen.getByRole("button", { name: "gamma.md" }));
    state.layoutReady = true;
    await settleFrames(4);

    expect(scrollToSpy).toHaveBeenCalledTimes(1);
    expect(scrollToSpy).toHaveBeenCalledWith({
      top: FILE_OFFSETS[LAST_FILE]! - DIFF_FILE_SCROLL_INSET_PX,
      behavior: "auto",
    });
    expect(screen.getByRole("button", { name: "gamma.md" })).toHaveAttribute(
      "aria-current",
      "page",
    );
  });

  it("hands the viewport back to the user when they scroll during a jump", async () => {
    installScrollGeometry({ layoutReady: false });
    renderTreeDiff();

    const beta = await screen.findByRole("button", { name: "beta.md" });
    fireEvent.click(beta);
    // Wheel input cancels the jump, so the following scroll event is the
    // user's and the spy must follow it instead of standing down.
    fireEvent.wheel(scrollRegion());
    fireEvent.scroll(scrollRegion());
    await settleFrames(2);

    expect(scrollToSpy).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "gamma.md" })).toHaveAttribute(
      "aria-current",
      "page",
    );
  });

  it("paints a click flash on the selected section and replays it on re-click", async () => {
    installScrollGeometry({ layoutReady: false });
    renderTreeDiff();

    const beta = await screen.findByRole("button", { name: "beta.md" });
    // No layout: the jump cannot scroll, but the click must still acknowledge.
    fireEvent.click(beta);

    const betaSection = () =>
      document.querySelector('[data-diff-path="docs/specs/beta.md"]');
    const gammaSection = () =>
      document.querySelector('[data-diff-path="docs/specs/gamma.md"]');

    expect(betaSection()?.querySelector(".ora-diff-file-flash")).not.toBeNull();
    expect(document.querySelectorAll(".ora-diff-file-flash")).toHaveLength(1);

    // Clicking another file moves the flash with it.
    fireEvent.click(screen.getByRole("button", { name: "gamma.md" }));
    expect(betaSection()?.querySelector(".ora-diff-file-flash")).toBeNull();
    const firstFlash = gammaSection()?.querySelector(".ora-diff-file-flash");
    expect(firstFlash).not.toBeNull();

    // Re-clicking the same file remounts the overlay, replaying the animation.
    fireEvent.click(screen.getByRole("button", { name: "gamma.md" }));
    const secondFlash = gammaSection()?.querySelector(".ora-diff-file-flash");
    expect(secondFlash).not.toBeNull();
    expect(secondFlash).not.toBe(firstFlash);
  });
});
