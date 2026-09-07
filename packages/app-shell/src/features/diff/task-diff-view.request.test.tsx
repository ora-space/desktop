import { createElement, type ReactNode } from "react";
import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
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

const FIRST_FILE = "docs/specs/2026-08-01-ora-spec-management-design.md";
const REQUESTED_FILE = "docs/specs/demo-spec.md";
const LAST_FILE = "docs/specs/test-spec.md";

const MULTI_FILE_PATCH = [
  addedFilePatch(FIRST_FILE, "# design"),
  addedFilePatch(REQUESTED_FILE, "# demo"),
  addedFilePatch(LAST_FILE, "# test"),
].join("");

/** Renders Changes with a mocked multi-file patch and an explicit file request. */
function renderRequestedDiff(fileRequest?: {
  path?: string;
  requestId?: number;
  line?: number;
}) {
  const clientHandlers: TestHandlers = {};
  const client = createTestClient(clientHandlers);
  clientHandlers.getWorkspaceDiff = async () => ({
    baseCommitId: "base",
    headCommitId: "head",
    patch: MULTI_FILE_PATCH,
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
      fileRequest={{
        path: fileRequest?.path ?? REQUESTED_FILE,
        requestId: fileRequest?.requestId ?? 1,
        line: fileRequest?.line,
      }}
      onFileTreeOpenChange={() => undefined}
    />,
    { wrapper },
  );
}

const REQUESTED_FILE_OFFSET = 800;
const DIFF_VIEWPORT_HEIGHT = 400;

/** Empty box used before the Changes panel has a real layout. */
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

describe("TaskDiffView file requests", () => {
  const nativeGetBoundingClientRect =
    HTMLElement.prototype.getBoundingClientRect;

  afterEach(() => {
    vi.restoreAllMocks();
    HTMLElement.prototype.getBoundingClientRect = nativeGetBoundingClientRect;
    Reflect.deleteProperty(HTMLElement.prototype, "scrollTo");
    Reflect.deleteProperty(HTMLElement.prototype, "clientHeight");
    Reflect.deleteProperty(HTMLElement.prototype, "offsetHeight");
    Reflect.deleteProperty(HTMLElement.prototype, "offsetTop");
    Reflect.deleteProperty(HTMLElement.prototype, "scrollTop");
    Reflect.deleteProperty(HTMLElement.prototype, "scrollLeft");
  });

  it("keeps the requested file selected after the changes list mounts", async () => {
    renderRequestedDiff();

    const requested = await screen.findByRole("button", {
      name: "demo-spec.md",
    });
    await waitFor(() => {
      expect(requested).toHaveAttribute("aria-current", "page");
    });
    expect(
      screen.getByRole("button", {
        name: "2026-08-01-ora-spec-management-design.md",
      }),
    ).not.toHaveAttribute("aria-current");
    expect(
      screen.getByRole("button", { name: "test-spec.md" }),
    ).not.toHaveAttribute("aria-current");
  });

  it("scrolls the requested file section into view after the panel lays out", async () => {
    let layoutReady = false;
    const scrollTo = vi.fn();

    Object.defineProperty(HTMLElement.prototype, "scrollTo", {
      configurable: true,
      writable: true,
      value(this: HTMLElement, arg?: ScrollToOptions | number) {
        if (
          this.classList.contains("ora-diff-scroll-region") &&
          typeof arg === "object"
        ) {
          scrollTo(arg);
        }
      },
    });

    Object.defineProperty(HTMLElement.prototype, "clientHeight", {
      configurable: true,
      get() {
        if (!this.classList?.contains("ora-diff-scroll-region")) return 0;
        return layoutReady ? DIFF_VIEWPORT_HEIGHT : 0;
      },
    });
    Object.defineProperty(HTMLElement.prototype, "offsetHeight", {
      configurable: true,
      get() {
        if (!layoutReady) return 0;
        if (this.getAttribute?.("data-diff-path") === REQUESTED_FILE)
          return 120;
        if (this.classList?.contains("ora-diff-scroll-region"))
          return DIFF_VIEWPORT_HEIGHT;
        return 100;
      },
    });
    Object.defineProperty(HTMLElement.prototype, "offsetTop", {
      configurable: true,
      get() {
        if (!layoutReady) return 0;
        return this.getAttribute?.("data-diff-path") === REQUESTED_FILE
          ? REQUESTED_FILE_OFFSET
          : 0;
      },
    });
    Object.defineProperty(HTMLElement.prototype, "scrollTop", {
      configurable: true,
      get() {
        return 0;
      },
    });
    HTMLElement.prototype.getBoundingClientRect = function () {
      if (!layoutReady) return emptyRect();
      if (this.classList.contains("ora-diff-scroll-region")) {
        return boxRect(0, 800, DIFF_VIEWPORT_HEIGHT);
      }
      if (this.getAttribute("data-diff-path") === REQUESTED_FILE) {
        return boxRect(REQUESTED_FILE_OFFSET, 800, 120);
      }
      return nativeGetBoundingClientRect.call(this);
    };

    renderRequestedDiff();

    const requested = await screen.findByRole("button", {
      name: "demo-spec.md",
    });
    await waitFor(() => {
      expect(requested).toHaveAttribute("aria-current", "page");
    });

    layoutReady = true;
    await act(async () => {
      await new Promise<void>((resolve) => {
        requestAnimationFrame(() => resolve());
      });
      await new Promise<void>((resolve) => {
        requestAnimationFrame(() => resolve());
      });
    });

    await waitFor(() => {
      expect(scrollTo).toHaveBeenCalledWith({
        top: REQUESTED_FILE_OFFSET - DIFF_FILE_SCROLL_INSET_PX,
        behavior: "auto",
      });
    });
  });

  it("centers the requested line without shifting the diff sideways", async () => {
    const scrollTo = vi.fn();
    Object.defineProperty(HTMLElement.prototype, "scrollTo", {
      configurable: true,
      writable: true,
      value(arg?: ScrollToOptions | number) {
        if (typeof arg === "object" && arg !== null) scrollTo(arg);
      },
    });
    // The scrollport reports a stable rounded geometry so the vertical center
    // is deterministic, while a non-zero scrollLeft proves it is preserved.
    Object.defineProperty(HTMLElement.prototype, "clientHeight", {
      configurable: true,
      get() {
        return this.classList?.contains("ora-diff-scroll-region")
          ? DIFF_VIEWPORT_HEIGHT
          : 0;
      },
    });
    Object.defineProperty(HTMLElement.prototype, "offsetHeight", {
      configurable: true,
      get() {
        return 120;
      },
    });
    Object.defineProperty(HTMLElement.prototype, "offsetTop", {
      configurable: true,
      get() {
        return 800;
      },
    });
    Object.defineProperty(HTMLElement.prototype, "scrollTop", {
      configurable: true,
      get() {
        return 0;
      },
    });
    Object.defineProperty(HTMLElement.prototype, "scrollLeft", {
      configurable: true,
      get() {
        return 120;
      },
    });
    Object.defineProperty(HTMLElement.prototype, "getBoundingClientRect", {
      configurable: true,
      value: function (this: HTMLElement) {
        if (this.classList?.contains("ora-diff-scroll-region")) {
          return boxRect(0, 800, DIFF_VIEWPORT_HEIGHT);
        }
        if (
          this.classList?.contains("diff-code-selected") ||
          this.classList?.contains("diff-selected")
        ) {
          return boxRect(1000, 800, 120);
        }
        return nativeGetBoundingClientRect.call(this);
      },
    });

    const patch = [
      "diff --git a/src/main.rs b/src/main.rs",
      "new file mode 100644",
      "index 0000000..1111111",
      "--- /dev/null",
      "+++ b/src/main.rs",
      "@@ -0,0 +1,3 @@",
      "+fn main() {",
      '+    println!("hi");',
      "+}",
      "",
    ].join("\n");
    const clientHandlers: TestHandlers = {};
    const client = createTestClient(clientHandlers);
    clientHandlers.getWorkspaceDiff = async () => ({
      baseCommitId: "base",
      headCommitId: "head",
      patch,
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
    const { container } = render(
      <TaskDiffView
        workspaceId="task-1"
        hasBaseline
        viewType="unified"
        fileTreeOpen
        fileRequest={{ path: "src/main.rs", requestId: 1, line: 2 }}
        onFileTreeOpenChange={() => undefined}
      />,
      { wrapper },
    );

    const requested = await screen.findByRole("button", { name: "main.rs" });
    await waitFor(() => {
      expect(requested).toHaveAttribute("aria-current", "page");
    });
    await waitFor(() => {
      expect(
        container.querySelector(".diff-code-selected, .diff-selected"),
      ).not.toBeNull();
    });
    // 1000 - 0 + 0 = 1000 content offset; center pulls it up by
    // (400 - 120) / 2 = 140 while keeping the original scrollLeft of 120.
    expect(scrollTo).toHaveBeenCalledWith({ top: 860, left: 120 });
  });

  it("highlights every new-side line in a cited range", async () => {
    const patch = [
      "diff --git a/src/main.rs b/src/main.rs",
      "new file mode 100644",
      "index 0000000..1111111",
      "--- /dev/null",
      "+++ b/src/main.rs",
      "@@ -0,0 +1,3 @@",
      "+fn main() {",
      '+    println!("hi");',
      "+}",
      "",
    ].join("\n");
    const clientHandlers: TestHandlers = {};
    const client = createTestClient(clientHandlers);
    clientHandlers.getWorkspaceDiff = async () => ({
      baseCommitId: "base",
      headCommitId: "head",
      patch,
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
    const { container } = render(
      <TaskDiffView
        workspaceId="task-1"
        hasBaseline
        viewType="unified"
        fileTreeOpen
        fileRequest={{
          path: "src/main.rs",
          requestId: 1,
          line: 1,
          endLine: 3,
        }}
        onFileTreeOpenChange={() => undefined}
      />,
      { wrapper },
    );

    await waitFor(() => {
      expect(
        container.querySelectorAll(".diff-code-selected, .diff-selected"),
      ).toHaveLength(3);
    });
  });

  it("highlights an old-side cited range on the delete rows", async () => {
    const patch = [
      "diff --git a/src/example.ts b/src/example.ts",
      "index 1111111..2222222 100644",
      "--- a/src/example.ts",
      "+++ b/src/example.ts",
      "@@ -1,3 +1,3 @@",
      " keep",
      "-removed",
      "+added",
      "",
    ].join("\n");
    const clientHandlers: TestHandlers = {};
    const client = createTestClient(clientHandlers);
    clientHandlers.getWorkspaceDiff = async () => ({
      baseCommitId: "base",
      headCommitId: "head",
      patch,
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
    const { container } = render(
      <TaskDiffView
        workspaceId="task-1"
        hasBaseline
        viewType="unified"
        fileTreeOpen
        fileRequest={{
          path: "src/example.ts",
          requestId: 1,
          line: 2,
          endLine: 2,
          side: "old",
        }}
        onFileTreeOpenChange={() => undefined}
      />,
      { wrapper },
    );

    await waitFor(() => {
      expect(
        container.querySelectorAll(".diff-code-selected, .diff-selected")
          .length,
      ).toBeGreaterThan(0);
    });
    const selectedText = [
      ...container.querySelectorAll(".diff-code-selected, .diff-selected"),
    ]
      .map((node) => node.textContent ?? "")
      .join("");
    expect(selectedText).toContain("removed");
    expect(selectedText).not.toContain("added");
  });

  it("clears the jump highlight when clicking another diff line", async () => {
    const patch = [
      "diff --git a/src/main.rs b/src/main.rs",
      "new file mode 100644",
      "index 0000000..1111111",
      "--- /dev/null",
      "+++ b/src/main.rs",
      "@@ -0,0 +1,3 @@",
      "+fn main() {",
      '+    println!("hi");',
      "+}",
      "",
    ].join("\n");
    const clientHandlers: TestHandlers = {};
    const client = createTestClient(clientHandlers);
    clientHandlers.getWorkspaceDiff = async () => ({
      baseCommitId: "base",
      headCommitId: "head",
      patch,
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
    const { container } = render(
      <TaskDiffView
        workspaceId="task-1"
        hasBaseline
        viewType="unified"
        fileTreeOpen
        fileRequest={{ path: "src/main.rs", requestId: 1, line: 2 }}
        onFileTreeOpenChange={() => undefined}
      />,
      { wrapper },
    );

    await waitFor(() => {
      expect(
        container.querySelector(".diff-code-selected, .diff-selected"),
      ).not.toBeNull();
    });
    const unselected = [...container.querySelectorAll(".diff-code")].find(
      (node) =>
        !node.classList.contains("diff-code-selected") &&
        !node.classList.contains("diff-selected"),
    );
    expect(unselected).toBeDefined();
    fireEvent.mouseDown(unselected!, { button: 0 });

    expect(
      container.querySelector(".diff-code-selected, .diff-selected"),
    ).toBeNull();
  });

  it("still selects the file when the requested line is absent from the patch", async () => {
    renderRequestedDiff({ line: 99 });

    const requested = await screen.findByRole("button", {
      name: "demo-spec.md",
    });
    await waitFor(() => {
      expect(requested).toHaveAttribute("aria-current", "page");
    });
    expect(
      document.querySelector(".diff-code-selected, .diff-selected"),
    ).toBeNull();
  });
});

describe("TaskDiffView collapsed context", () => {
  it("expands a collapsed unchanged block without crashing", async () => {
    const clientHandlers: TestHandlers = {};
    const client = createTestClient(clientHandlers);
    const longContextPatch = [
      "diff --git a/src/example.ts b/src/example.ts",
      "index 1111111..2222222 100644",
      "--- a/src/example.ts",
      "+++ b/src/example.ts",
      "@@ -1,20 +1,20 @@",
      ...Array.from({ length: 9 }, (_, index) => ` line ${index + 1}`),
      "-const value = 10;",
      "+const value = 20;",
      ...Array.from({ length: 10 }, (_, index) => ` line ${index + 11}`),
      "",
    ].join("\n");
    clientHandlers.getWorkspaceDiff = async () => ({
      baseCommitId: "base",
      headCommitId: "head",
      patch: longContextPatch,
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

    render(
      <TaskDiffView
        workspaceId="task-1"
        hasBaseline
        viewType="unified"
        fileTreeOpen={false}
        onFileTreeOpenChange={() => undefined}
      />,
      { wrapper },
    );

    const expandButtons = await screen.findAllByRole("button", {
      name: /展开 \d+ 行未修改内容|Expand \d+ unchanged lines/,
    });
    expect(expandButtons.length).toBeGreaterThan(0);
    await act(async () => {
      expandButtons[0]!.click();
    });
    await waitFor(() => {
      expect(
        screen.queryAllByRole("button", {
          name: /展开 \d+ 行未修改内容|Expand \d+ unchanged lines/,
        }).length,
      ).toBeLessThan(expandButtons.length);
    });
  });
});
