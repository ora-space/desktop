import { createElement, type ReactNode } from "react";
import { render, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, describe, expect, it, vi } from "vitest";
import { AppI18nProvider } from "../../i18n/i18n";
import { ContractsClientContext } from "../../contracts-client-context";
import {
  createTestClient,
  type TestHandlers,
} from "../../test/contracts-transport";
import "../../i18n/i18n-instance";
import { diffKeys } from "../../state/data/diff";
import { TaskDiffView } from "./task-diff-view";

/**
 * Gives the row virtualizer a viewport size in jsdom. jsdom performs no
 * layout, so every element reports zero offsetHeight and the window would
 * compute as empty; @tanstack/react-virtual reads offsetHeight for its rect.
 */
const mockViewportSize = (height: number) => {
  vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockReturnValue(
    height,
  );
  vi.spyOn(HTMLElement.prototype, "offsetWidth", "get").mockReturnValue(800);
};

/** Builds a new-file patch whose single hunk carries `lines` added lines. */
function largeFilePatch(path: string, lines: number): string {
  const body = Array.from({ length: lines }, (_line, index) => `+${index + 1}`);
  return [
    `diff --git a/${path} b/${path}`,
    "new file mode 100644",
    "index 0000000..1111111",
    "--- /dev/null",
    `+++ b/${path}`,
    `@@ -0,0 +1,${lines} @@`,
    ...body,
    "",
  ].join("\n");
}

/**
 * Renders Changes with the patch pre-seeded into the query cache, so the very
 * first commit mounts the file (the defer decision happens there) — mirroring
 * a session switch back onto an already-loaded workspace diff.
 */
function renderDiff(patch: string) {
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
  queryClient.setQueryData(diffKeys.workspaceDiff("task-1", "branch"), {
    baseCommitId: "base",
    headCommitId: "head",
    patch,
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

afterEach(() => vi.restoreAllMocks());

describe("TaskDiffView deferred kinds", () => {
  it("auto-focuses and chunk-windows a single oversized file", async () => {
    mockViewportSize(600);
    const { container } = renderDiff(largeFilePatch("big/generated.ts", 700));

    await waitFor(() =>
      expect(container.querySelector(".diff-line")).not.toBeNull(),
    );
    // Every chunk is a real native react-diff-view table; only a bounded
    // window of them is mounted (far fewer than the file's chunk count).
    expect(container.querySelector("table.diff")).not.toBeNull();
    const mounted = container.querySelectorAll("table.diff").length;
    expect(mounted).toBeGreaterThan(0);
    expect(mounted).toBeLessThan(6);
    // The windowed rows keep the insert tint.
    expect(container.querySelector(".diff-code-insert")).not.toBeNull();
  });

  it("renders small diffs immediately without windowing", async () => {
    const { container } = renderDiff(largeFilePatch("src/small.ts", 5));

    await waitFor(() =>
      expect(container.querySelector(".diff-line")).not.toBeNull(),
    );
    // A small file stays as a single native table.
    const mounted = container.querySelectorAll("table.diff").length;
    expect(mounted).toBeGreaterThan(0);
    expect(mounted).toBeLessThan(3);
  });
});
