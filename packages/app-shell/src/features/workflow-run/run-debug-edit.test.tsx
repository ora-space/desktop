import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { createChatStore } from "@ora/chat";
import {
  RemoteContractError,
  type GetWorkflowDebugContextRequest,
  type WorkflowDebugTarget,
} from "@ora/contracts";
import { appI18n } from "../../i18n/i18n-instance";
import {
  createHookWrapper,
  createTestQueryClient,
} from "../../test/hook-harness";
import {
  createTestClient,
  type OperationHandler,
} from "../../test/contracts-transport";
import { RunActInspector } from "./run-act-inspector";

vi.mock("../../state/hooks/use-workflow-agent-models", () => ({
  useWorkflowAgentModels: () => ({ agentModels: [] }),
}));

beforeEach(async () => {
  await appI18n.changeLanguage("en-US");
});

const graph = JSON.stringify({
  nodes: [
    {
      id: "loop",
      data: {
        kind: "loop",
        title: "Loop",
        loopConfig: {
          until: { conditions: [], logic: "and" },
          maxIterations: 5,
        },
      },
    },
  ],
  edges: [],
});
const physical = { runId: "F", nodeRunId: "attempt" };
const logical = {
  runId: "F",
  pendingTarget: { rootScopeId: "root", scopeId: "root", nodeId: "loop" },
};

function setup(
  selection: GetWorkflowDebugContextRequest,
  runStatus: "running" | "pending" = "running",
) {
  const user = userEvent.setup();
  const target: WorkflowDebugTarget = {
    nodeId: "loop",
    scopeId: "root",
    round: null,
    index: null,
    containerNodeRunId: null,
    ...("nodeRunId" in selection
      ? { nodeRunId: selection.nodeRunId }
      : { rootScopeId: "root" }),
  };
  const context = vi
    .fn<OperationHandler<"getWorkflowDebugContext">>()
    .mockResolvedValue({ baseRevision: 5, target, graph });
  const preview = vi
    .fn<OperationHandler<"previewWorkflowDebug">>()
    .mockResolvedValue({
      previewId: "p",
      baseRevision: 5,
      target,
      keep: [],
      stop: [],
      invalidate: ["old"],
      schedule: ["loop"],
      predictedBranch: null,
      workspace: "keep",
      externalActionsMayRepeat: true,
    });
  const commit = vi
    .fn<OperationHandler<"commitWorkflowDebug">>()
    .mockRejectedValueOnce(new Error("reply lost"))
    .mockImplementation(async ({ requestId }) => ({
      operationId: "op",
      requestId,
      baseRevision: 5,
      state: "applied",
      revision: 6,
    }));
  const history = vi
    .fn<OperationHandler<"getWorkflowDebugHistory">>()
    .mockResolvedValue({ entries: [] });
  const client = createTestClient({
    getWorkflowDebugContext: context,
    previewWorkflowDebug: preview,
    commitWorkflowDebug: commit,
    getWorkflowDebugHistory: history,
  });
  const wrapper = createHookWrapper(
    client,
    createTestQueryClient(),
    createChatStore(client.session),
  );
  const inspector = (selection: GetWorkflowDebugContextRequest) => (
    <RunActInspector
      runId={selection.runId}
      runStatus={runStatus}
      nodeId="loop"
      data={{ kind: "loop", title: "Loop", description: "" }}
      state={
        "nodeRunId" in selection
          ? { status: "succeeded", nodeRunId: selection.nodeRunId }
          : { status: "inactive" }
      }
      pendingPosition={
        "pendingTarget" in selection ? selection.pendingTarget : undefined
      }
      selectedRound={null}
      onRoundChange={vi.fn()}
      artifacts={[]}
      revealedArtifactId={null}
    />
  );
  const view = render(inspector(selection), { wrapper });
  const open = () =>
    user.click(
      screen.getByRole("button", { name: "Edit and resume from here" }),
    );
  async function previewEdit() {
    await open();
    fireEvent.change(await screen.findByRole("spinbutton"), {
      target: { value: "9" },
    });
    await user.click(screen.getByRole("button", { name: "Preview changes" }));
    return screen.findByRole("button", { name: "Confirm and resume" });
  }
  return {
    user,
    history,
    context,
    preview,
    commit,
    open,
    previewEdit,
    select: (next: GetWorkflowDebugContextRequest) =>
      view.rerender(inspector(next)),
  };
}

it("keeps a moved draft editable and can close it before any preview", async () => {
  const { user, open, select, context, preview } = setup(physical);
  await open();
  fireEvent.change(await screen.findByRole("spinbutton"), {
    target: { value: "9" },
  });
  select({ runId: "F", nodeRunId: "next" });
  expect(screen.getByRole("status")).toHaveTextContent(
    /draft still edits the original execution/i,
  );
  expect(screen.getByRole("spinbutton")).toHaveValue(9);
  expect(context).toHaveBeenCalledTimes(1);
  select(physical);
  expect(screen.queryByRole("status")).toBeNull();
  expect(screen.getByRole("spinbutton")).toHaveValue(9);
  select({ runId: "F", nodeRunId: "next" });
  await user.click(screen.getByRole("button", { name: "Close editor" }));
  expect(
    screen.getByRole("button", { name: "Edit and resume from here" }),
  ).toBeVisible();
  expect(preview).not.toHaveBeenCalled();
});

it("reloads context when the preview itself is stale", async () => {
  const { user, open, preview, context, commit } = setup(physical);
  await open();
  fireEvent.change(await screen.findByRole("spinbutton"), {
    target: { value: "9" },
  });
  const initial = await context.mock.results[0]!.value;
  context.mockResolvedValue({
    ...initial,
    baseRevision: 6,
    graph: graph.replace('"maxIterations":5', '"maxIterations":7'),
  });
  preview.mockRejectedValueOnce(
    new RemoteContractError(
      { code: "workflow_debug_stale_preview", params: {}, requestId: "stale" },
      {},
    ),
  );
  await user.click(screen.getByRole("button", { name: "Preview changes" }));
  expect(await screen.findByRole("alert")).toHaveTextContent(
    /preview is stale/i,
  );
  await waitFor(() => expect(screen.getByRole("spinbutton")).toHaveValue(7));
  expect(context.mock.calls.map(([request]) => request)).toEqual([
    physical,
    physical,
  ]);
  expect(commit).not.toHaveBeenCalled();
});

it("can close and reselect after the server rejects an in-flight confirmation for a moved selection", async () => {
  const { user, previewEdit, select, commit } = setup(physical);
  const confirm = await previewEdit();
  let rejectCommit!: (reason: Error) => void;
  commit.mockReset().mockImplementationOnce(
    () =>
      new Promise((_resolve, reject) => {
        rejectCommit = reject;
      }),
  );
  await user.click(confirm);
  select({ runId: "F", nodeRunId: "next" });
  await act(async () =>
    rejectCommit(
      new RemoteContractError(
        {
          code: "workflow_debug_stale_preview",
          params: {},
          requestId: "rejected",
        },
        {},
      ),
    ),
  );
  expect(await screen.findByRole("alert")).toHaveTextContent(
    /preview is stale/i,
  );
  expect(screen.getByRole("status")).toHaveTextContent(
    /close the editor, select a position/i,
  );
  expect(screen.queryByRole("spinbutton")).toBeNull();
  expect(
    screen.queryByRole("button", { name: "Confirm and resume" }),
  ).toBeNull();
  await user.click(screen.getByRole("button", { name: "Close editor" }));
  expect(
    screen.getByRole("button", { name: "Edit and resume from here" }),
  ).toBeVisible();
});

it("keeps the editor unavailable on a pending run even with a pending position", () => {
  const { context } = setup(logical, "pending");
  expect(
    screen.queryByRole("button", { name: "Edit and resume from here" }),
  ).toBeNull();
  expect(context).not.toHaveBeenCalled();
});

it.each([physical, logical])(
  "keeps the retry and receipt across refreshed attempts for %j",
  async (selection: GetWorkflowDebugContextRequest) => {
    const { user, context, preview, commit, open, select } = setup(selection);
    await open();
    fireEvent.change(await screen.findByRole("spinbutton"), {
      target: { value: "8" },
    });
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(preview).not.toHaveBeenCalled();
    await open();
    expect(await screen.findByRole("spinbutton")).toHaveValue(5);
    expect(context.mock.calls.at(-1)?.[0]).toEqual(selection);
    fireEvent.change(screen.getByRole("spinbutton"), {
      target: { value: "9" },
    });
    await user.click(screen.getByRole("button", { name: "Preview changes" }));
    expect(await screen.findByText("old")).toBeVisible();
    expect(preview.mock.calls[0]?.[0]).toEqual({
      ...selection,
      baseRevision: 5,
      patch: [{ op: "replace", path: "/loopConfig/maxIterations", value: 9 }],
    });
    expect(commit).not.toHaveBeenCalled();
    const confirm = screen.getByRole("button", { name: "Confirm and resume" });
    await user.click(confirm);
    expect(await screen.findByRole("alert")).toHaveTextContent(
      /Retry this request/,
    );
    select({ runId: "F", nodeRunId: "new-attempt" });
    await user.click(
      screen.getByRole("button", { name: "Confirm and resume" }),
    );
    await waitFor(() => expect(commit).toHaveBeenCalledTimes(2));
    expect(commit.mock.calls.map(([request]) => request)).toEqual([
      { runId: "F", previewId: "p", requestId: expect.any(String) },
      commit.mock.calls[0]![0],
    ]);
    expect(await screen.findByText(/revision 6/i)).toBeVisible();
    select({ runId: "F", nodeRunId: "another-attempt" });
    expect(screen.getByText(/revision 6/i)).toBeVisible();
  },
);

it("requires a new preview after leaving an unconfirmed target", async () => {
  const { user, previewEdit, select, commit, context, open } = setup(physical);
  await previewEdit();
  const next = { runId: "F", nodeRunId: "other" };
  select(next);
  expect(await screen.findByText(/selected execution changed/i)).toBeVisible();
  select(physical);
  expect(
    screen.queryByRole("button", { name: "Confirm and resume" }),
  ).toBeNull();
  expect(commit).not.toHaveBeenCalled();
  select(next);
  await user.click(screen.getByRole("button", { name: "Close editor" }));
  await open();
  await screen.findByRole("spinbutton");
  expect(context.mock.calls.at(-1)?.[0]).toEqual(next);
});

it.each([
  ["workflow_debug_stale_preview", /preview is stale/i],
  ["workflow_debug_request_conflict", /key already names another preview/i],
] as const)(
  "discards a rejected ticket and reloads context for %s",
  async (code, message) => {
    const { user, previewEdit, commit, context } = setup(physical);
    const confirm = await previewEdit();
    commit
      .mockReset()
      .mockRejectedValue(
        new RemoteContractError({ code, params: {}, requestId: "reject" }, {}),
      );
    context.mockResolvedValue({
      baseRevision: 6,
      target: {
        nodeRunId: "attempt",
        nodeId: "loop",
        scopeId: "root",
        round: null,
        index: null,
        containerNodeRunId: null,
      },
      graph: graph.replace('"maxIterations":5', '"maxIterations":7'),
    });
    await user.click(confirm);
    expect(await screen.findByRole("alert")).toHaveTextContent(message);
    expect(
      screen.queryByRole("button", { name: "Confirm and resume" }),
    ).toBeNull();
    expect(await screen.findByRole("spinbutton")).toHaveValue(7);
    expect(context.mock.calls.map(([req]) => req)).toEqual([
      physical,
      physical,
    ]);
  },
);

it("recovers a lost confirmation by request ID after first dispatch", async () => {
  const { user, previewEdit, commit, preview, history, select } =
    setup(logical);
  const confirm = await previewEdit();
  const acceptedPreview = await preview.mock.results[0]!.value;
  let loseReply!: () => void;
  commit.mockReset().mockImplementationOnce(
    () =>
      new Promise((_resolve, reject) => {
        loseReply = () => reject(new Error("reply lost"));
      }),
  );
  history.mockImplementation(async () => {
    const operation = {
      baseRevision: 5,
      revision: 6,
      requestId: commit.mock.calls[0]![0].requestId,
      operationId: "matching",
      state: "applied" as const,
    };
    const entry = {
      firstNodeRunId: "first",
      priorError: null,
      priorRunStatus: "running" as const,
      operation,
      preview: acceptedPreview,
      patch: [],
    };
    return {
      entries: [
        {
          ...entry,
          operation: { ...operation, requestId: "different", revision: 4 },
        },
        entry,
      ],
    };
  });
  await user.click(confirm);
  select({ runId: "F", nodeRunId: "first" });
  await act(async () => loseReply());
  expect(await screen.findByText(/revision 6/i)).toBeVisible();
  expect(screen.queryByText(/retry this request/i)).toBeNull();
  expect(commit).toHaveBeenCalledTimes(1);
});
