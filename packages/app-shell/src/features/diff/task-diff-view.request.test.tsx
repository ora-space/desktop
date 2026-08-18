import { createElement, type ReactNode } from "react";
import { render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { describe, expect, it } from "vitest";
import { AppI18nProvider } from "../../i18n/i18n";
import { ContractsClientContext } from "../../contracts-client-context";
import {
  createMockClient,
  createMockClientState,
} from "../../test/mock-client";
import { TaskDiffView } from "./task-diff-view";

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
function renderRequestedDiff() {
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
      fileRequest={{ path: REQUESTED_FILE, requestId: 1 }}
      onFileTreeOpenChange={() => undefined}
    />,
    { wrapper },
  );
}

describe("TaskDiffView file requests", () => {
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
});
