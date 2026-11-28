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
import { decodeRemoteError } from "@ora/contracts";
import refusals from "./fixtures/debug-refusals.json";
import { RunDebugEdit } from "./run-debug-edit";
import type {
  GetWorkflowDebugContextRequest,
  WorkflowDebugTarget,
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

it.each([physical, logical])(
  "previews only on submit and retries the same confirmation from %j",
  async (selection: GetWorkflowDebugContextRequest) => {
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
    const client = createTestClient({
      getWorkflowDebugContext: context,
      previewWorkflowDebug: preview,
      commitWorkflowDebug: commit,
      getWorkflowDebugHistory: async () => ({ entries: [] }),
    });
    const wrapper = createHookWrapper(
      client,
      createTestQueryClient(),
      createChatStore(client.session),
    );
    render(
      <RunActInspector
        runId={selection.runId}
        runStatus="running"
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
      />,
      { wrapper },
    );
    const open = () =>
      user.click(
        screen.getByRole("button", { name: "Edit and resume from here" }),
      );
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
    await user.click(confirm);
    await waitFor(() => expect(commit).toHaveBeenCalledTimes(2));
    expect(commit.mock.calls.map(([request]) => request)).toEqual([
      { runId: "F", previewId: "p", requestId: expect.any(String) },
      commit.mock.calls[0]![0],
    ]);
    expect(await screen.findByText(/revision 6/i)).toBeVisible();
  },
);

// Rust verifies these envelopes against SQLite-backed coordinator.preview calls.
it.each([
  [0, "作用域 root:run-1 中的节点 out 尚不能首次派发。请检查其前置节点（x）。"],
  [1, "请确认变量 absent.value 已声明，且在所选位置有记录值。"],
  [2, "请选择本次运行的当前尝试或可到达的位置。"],
  [3, "请输入文本。"],
] as const)(
  "renders refusal %i again when the open editor changes language",
  async (index, chinese) => {
    const response = refusals[index]!;
    const user = userEvent.setup();
    const preview = vi
      .fn<OperationHandler<"previewWorkflowDebug">>()
      .mockRejectedValue(decodeRemoteError(response));
    const client = createTestClient({
      getWorkflowDebugContext: async () => ({
        graph,
        baseRevision: 0,
        target: {
          nodeRunId: physical.nodeRunId,
          nodeId: "loop",
          scopeId: "root",
          round: null,
          index: null,
          containerNodeRunId: null,
        },
      }),
      getWorkflowDebugHistory: async () => ({ entries: [] }),
      previewWorkflowDebug: preview,
    });
    const wrapper = createHookWrapper(
      client,
      createTestQueryClient(),
      createChatStore(client.session),
    );
    render(<RunDebugEdit target={physical} />, { wrapper });
    await user.click(
      screen.getByRole("button", { name: "Edit and resume from here" }),
    );
    fireEvent.change(await screen.findByRole("spinbutton"), {
      target: { value: "8" },
    });
    await user.click(screen.getByRole("button", { name: "Preview changes" }));
    const alert = await screen.findByRole("alert");
    const path = response.params.path;
    expect(alert).toHaveTextContent(
      path
        ? `This edit was rejected at ${path}: ${response.params.reason}`
        : `The selected execution cannot be debugged: ${response.params.reason}`,
    );
    await act(() => appI18n.changeLanguage("zh-CN").then(() => undefined));
    expect(alert).toHaveTextContent(
      path
        ? `这处修改未被接受（${path}）：${chinese}`
        : `无法调试选中的执行：${chinese}`,
    );
    expect(preview).toHaveBeenCalledTimes(1);
  },
);
