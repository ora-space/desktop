import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { createChatStore } from "@ora/chat";
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
