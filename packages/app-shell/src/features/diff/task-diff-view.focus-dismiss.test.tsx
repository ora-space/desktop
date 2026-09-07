import { createElement, type ReactNode } from "react";
import { fireEvent, render, waitFor } from "@testing-library/react";
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

/** Gives the virtualizer a viewport size in jsdom (no layout is performed). */
const mockViewportSize = (height: number) => {
  vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockReturnValue(
    height,
  );
  vi.spyOn(HTMLElement.prototype, "offsetWidth", "get").mockReturnValue(800);
};

afterEach(() => vi.restoreAllMocks());

/** A large single-file patch (500 lines) so focus mode is entered (>400). */
function bigFilePatch(): string {
  const body = Array.from({ length: 500 }, (_l, i) => `+line ${i + 1}`);
  return [
    "diff --git a/src/big.ts b/src/big.ts",
    "new file mode 100644",
    "index 0000000..1111111",
    "--- /dev/null",
    "+++ b/src/big.ts",
    `@@ -0,0 +1,500 @@`,
    ...body,
    "",
  ].join("\n");
}

describe("TaskDiffView focus jump dismiss", () => {
  it("clears the jump highlight when clicking a non-cited line in focus mode", async () => {
    mockViewportSize(600);
    const patch = bigFilePatch();
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
    const { container } = render(
      <TaskDiffView
        workspaceId="task-1"
        hasBaseline
        viewType="unified"
        fileTreeOpen
        fileRequest={{ path: "src/big.ts", requestId: 1, line: 480 }}
        onFileTreeOpenChange={() => undefined}
      />,
      { wrapper },
    );

    // A single file above the threshold enters focus mode (single article).
    await waitFor(() =>
      expect(container.querySelector("article")).not.toBeNull(),
    );
    // The cited line (480, in a later chunk) is highlighted.
    await waitFor(
      () =>
        expect(container.querySelector(".diff-code-selected")).not.toBeNull(),
      { timeout: 4000 },
    );

    // Click a non-cited diff line → the jump wash should clear.
    const other = [...container.querySelectorAll(".diff-code")].find(
      (node) =>
        !node.classList.contains("diff-code-selected") &&
        !node.classList.contains("diff-selected"),
    );
    expect(other).toBeDefined();
    fireEvent.mouseDown(other!, { button: 0 });

    await waitFor(() =>
      expect(container.querySelector(".diff-code-selected")).toBeNull(),
    );
  });
});
