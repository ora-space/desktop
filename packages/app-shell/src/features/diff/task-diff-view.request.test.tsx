import { createElement, type ReactNode } from "react";
import { act, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, describe, expect, it, vi } from "vitest";
import { AppI18nProvider } from "../../i18n/i18n";
import { ContractsClientContext } from "../../contracts-client-context";
import {
  createMockClient,
  createMockClientState,
} from "../../test/mock-client";
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
  const client = createMockClient(createMockClientState());
  client.task.getDiff = async () => ({
    baseCommitId: "base",
    headCommitId: "head",
    diffId: "diff-1",
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
      taskId="task-1"
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

  it("highlights and scrolls the requested new-side line", async () => {
    const scrollIntoView = vi.spyOn(Element.prototype, "scrollIntoView");
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
    const client = createMockClient(createMockClientState());
    client.task.getDiff = async () => ({
      baseCommitId: "base",
      headCommitId: "head",
      diffId: "diff-1",
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
        taskId="task-1"
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
    expect(scrollIntoView).toHaveBeenCalledWith({
      block: "center",
      inline: "nearest",
    });
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
