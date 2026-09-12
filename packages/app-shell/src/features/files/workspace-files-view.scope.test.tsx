import type { ReactNode } from "react";
import { act, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { I18nextProvider } from "react-i18next";
import { expect, it } from "vitest";
import { appI18n } from "../../i18n/i18n-instance";
import { ContractsClientContext } from "../../contracts-client-context";
import { createTestClient } from "../../test/contracts-transport";
import { fileKeys } from "../../state/data/files";
import { PlatformProvider } from "../../platform";
import { createStubPlatform } from "../../test/stub-platform";
import { WorkspaceFilesView } from "./workspace-files-view";

it("releases the old file stream on workspace switch and keeps cached listings isolated", async () => {
  const client = createTestClient({
    listWorkspaces: () => ({ workspaces: [] }),
    watchWorkspace: async function* ({ taskId }, options) {
      const signal = options?.signal;
      if (!signal) throw new Error("Files must own a cancellable stream");
      streams.push({ taskId, signal });
      try {
        await new Promise<void>((resolve) => {
          if (signal.aborted) resolve();
          else
            signal.addEventListener("abort", () => resolve(), { once: true });
        });
        yield* [];
      } finally {
        finished.push(taskId);
      }
    },
    listWorkspaceDirectory: async ({ taskId }) => ({
      path: "",
      entries: [
        {
          name: `${taskId}.rs`,
          path: `${taskId}.rs`,
          kind: "file",
          isSymbolicLink: false,
        },
      ],
    }),
    getTaskWorkspace: async ({ taskId }) => ({
      workspace: { rootPath: `/repo/${taskId}`, branchName: `task/${taskId}` },
    }),
  });
  const streams: Array<{ taskId: string; signal: AbortSignal }> = [];
  const finished: string[] = [];
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: Infinity } },
  });
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={queryClient}>
      <ContractsClientContext.Provider value={client}>
        <I18nextProvider i18n={appI18n}>
          <PlatformProvider adapter={createStubPlatform()}>
            {children}
          </PlatformProvider>
        </I18nextProvider>
      </ContractsClientContext.Provider>
    </QueryClientProvider>
  );
  const view = render(
    <WorkspaceFilesView projectId="p1" taskId="t1" hideHeader />,
    { wrapper },
  );
  expect(await screen.findByText("t1.rs")).toBeInTheDocument();
  expect(streams.map(({ taskId, signal }) => [taskId, signal.aborted])).toEqual(
    [["t1", false]],
  );

  view.rerender(<WorkspaceFilesView projectId="p1" taskId="t2" hideHeader />);
  expect(await screen.findByText("t2.rs")).toBeInTheDocument();
  expect(screen.queryByText("t1.rs")).not.toBeInTheDocument();
  await waitFor(() => expect(finished).toEqual(["t1"]));
  expect(streams.map(({ taskId, signal }) => [taskId, signal.aborted])).toEqual(
    [
      ["t1", true],
      ["t2", false],
    ],
  );
  expect(
    ["t1", "t2"].map((taskId) =>
      queryClient.getQueryData(fileKeys.workspaceDirectory(taskId, "")),
    ),
  ).toEqual([
    {
      path: "",
      entries: [
        { name: "t1.rs", path: "t1.rs", kind: "file", isSymbolicLink: false },
      ],
    },
    {
      path: "",
      entries: [
        { name: "t2.rs", path: "t2.rs", kind: "file", isSymbolicLink: false },
      ],
    },
  ]);

  await act(async () => view.unmount());
  await waitFor(() => expect(finished).toEqual(["t1", "t2"]));
  expect(streams.every(({ signal }) => signal.aborted)).toBe(true);
  queryClient.clear();
});
