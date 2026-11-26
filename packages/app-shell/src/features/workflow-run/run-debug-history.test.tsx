import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import {
  type WorkflowDebugPreview,
  type GetWorkflowDebugAttemptResponse,
} from "@ora/contracts";
import { createChatStore } from "@ora/chat";
import { appI18n } from "../../i18n/i18n-instance";
import {
  createHookWrapper,
  createTestQueryClient,
  renderHookWithClient,
} from "../../test/hook-harness";
import { createTestClient } from "../../test/contracts-transport";
import {
  invalidateWorkflowRun,
  useWorkflowDebugHistory,
} from "../../state/data/workflow-runs";
import { RunDebugHistory } from "./run-debug-history";

beforeEach(async () => {
  await appI18n.changeLanguage("en-US");
});

const preview: WorkflowDebugPreview = {
  previewId: "p1",
  baseRevision: 0,
  target: {
    nodeRunId: "it1",
    nodeId: "loop",
    scopeId: "root",
    round: null,
    index: null,
    containerNodeRunId: null,
  },
  keep: ["start.input"],
  invalidate: ["loop.export"],
  stop: ["attempt-3"],
  schedule: ["loop.entry"],
  workspace: "keep",
  externalActionsMayRepeat: true,
  predictedBranch: null,
  memberConfigurations: { worker: { prompt: "original" } },
};

it("re-reads durable history after a run invalidation through the generated client", async () => {
  const entry = {
    preview,
    patch: [],
    firstNodeRunId: null,
    priorRunStatus: "running" as const,
    priorError: null,
    operation: {
      state: "applied" as const,
      revision: 1,
      operationId: "op1",
      requestId: "k1",
      baseRevision: 0,
    },
  };
  const history = vi
    .fn()
    .mockResolvedValueOnce({ entries: [] })
    .mockResolvedValueOnce({ entries: [entry] });
  const view = renderHookWithClient(
    () => useWorkflowDebugHistory("F"),
    createTestClient({ getWorkflowDebugHistory: history }),
  );
  await waitFor(() => expect(view.result.current.data).toEqual([]));
  act(() => invalidateWorkflowRun(view.queryClient, "F"));
  await waitFor(() => expect(view.result.current.data).toEqual([entry]));
  expect(history.mock.calls.map((call) => call[0])).toEqual([
    { runId: "F" },
    { runId: "F" },
  ]);
  view.unmount();
});

it("opens the stored dispatch configuration from a reloaded run's history", async () => {
  const user = userEvent.setup();
  const entry = {
    preview: { ...preview, inputSourceNodeRunId: "earlier-input" },
    patch: [],
    firstNodeRunId: null,
    priorError: null,
    priorRunStatus: "running" as const,
    operation: {
      state: "applied" as const,
      revision: 1,
      operationId: "op1",
      requestId: "k1",
      baseRevision: 0,
    },
  };
  const attempt = vi.fn(
    async ({
      nodeRunId,
    }: {
      nodeRunId: string;
    }): Promise<GetWorkflowDebugAttemptResponse> => ({
      target: { ...preview.target, nodeRunId },
      status: "succeeded",
      revision: nodeRunId === "it1" ? 0 : null,
      configuration: nodeRunId === "it1" ? { maxIterations: 5 } : null,
      snapshotId: null,
      selectedBranch: null,
      superseded: nodeRunId === "it1",
      retiredByOperationId: nodeRunId === "it1" ? "op1" : null,
      inputRecorded: true,
      output: null,
      error: null,
    }),
  );
  const client = createTestClient({
    getWorkflowDebugHistory: async () => ({
      entries: [
        entry,
        {
          ...entry,
          preview: {
            ...preview,
            baseRevision: 1,
            target: { ...preview.target, nodeRunId: undefined },
          },
          firstNodeRunId: "first-new",
          operation: {
            ...entry.operation,
            operationId: "op2",
            requestId: "k2",
            baseRevision: 1,
            revision: 2,
          },
        },
      ],
    }),
    getWorkflowDebugAttempt: attempt,
  });
  const wrapper = createHookWrapper(
    client,
    createTestQueryClient(),
    createChatStore(client.session),
  );
  render(<RunDebugHistory runId="F" />, { wrapper });
  await user.click(screen.getByText("Debug history for this run"));
  await user.click(await screen.findByText(/loop · op1/));
  expect(attempt).not.toHaveBeenCalled();
  await user.click(screen.getByText("Inspect execution it1"));
  expect(await screen.findByText(/"maxIterations": 5/)).toBeVisible();
  expect(screen.getByText(/This attempt was superseded/)).toBeVisible();
  expect(
    screen.getByText("Execution supplying the dispatch input"),
  ).toBeVisible();
  await user.click(screen.getByText("Inspect execution earlier-input"));
  await waitFor(() => expect(attempt).toHaveBeenCalledTimes(2));
  await user.click(screen.getByText(/loop · op2/));
  const first = screen.getByText("Inspect execution first-new");
  await user.click(first);
  const newAttempt = within(first.closest("details")!);
  expect(
    await newAttempt.findByText(
      "This execution has no recorded configuration.",
    ),
  ).toBeVisible();
  expect(newAttempt.queryByText(/maxIterations/)).toBeNull();
  expect(attempt.mock.calls.map(([request]) => request)).toEqual([
    { runId: "F", nodeRunId: "it1" },
    { runId: "F", nodeRunId: "earlier-input" },
    { runId: "F", nodeRunId: "first-new" },
  ]);
});
