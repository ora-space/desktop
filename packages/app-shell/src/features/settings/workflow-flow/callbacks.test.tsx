import { memo } from "react";
import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import {
  WorkflowFlowCallbacksProvider,
  useWorkflowFlowActions,
  useWorkflowFlowConnectionState,
  type WorkflowFlowCallbacks,
} from "./callbacks";

/** Tracks action-context renders while exposing one callback for a semantic assertion. */
const ActionConsumer = memo(function ActionConsumer({
  onRender,
}: {
  onRender: () => void;
}) {
  onRender();
  const { onDeleteNode } = useWorkflowFlowActions();
  return <button type="button" onClick={() => onDeleteNode("node-1")}>delete</button>;
});

/** Tracks connection-context renders independently from stable graph actions. */
const ConnectionConsumer = memo(function ConnectionConsumer({
  onRender,
}: {
  onRender: () => void;
}) {
  onRender();
  const { connectionCandidateNodeId } = useWorkflowFlowConnectionState();
  return <span>{connectionCandidateNodeId ?? "none"}</span>;
});

describe("WorkflowFlowCallbacksProvider", () => {
  it("does not rerender action-only edge consumers when the candidate changes", () => {
    const onDeleteNode = vi.fn();
    const actions = {
      onDeleteNode,
      onDeleteEdge: vi.fn(),
      onSelectEdge: vi.fn(),
    };
    const actionRender = vi.fn();
    const connectionRender = vi.fn();

    /** Renders both consumer classes with a chosen transient candidate. */
    function view(candidateNodeId: string | null) {
      const value: WorkflowFlowCallbacks = {
        ...actions,
        connectionCandidateEndpoint: candidateNodeId === null ? null : "target",
        connectionCandidateNodeId: candidateNodeId,
      };
      return (
        <WorkflowFlowCallbacksProvider value={value}>
          <ActionConsumer onRender={actionRender} />
          <ConnectionConsumer onRender={connectionRender} />
        </WorkflowFlowCallbacksProvider>
      );
    }

    const { rerender } = render(view(null));
    rerender(view("node-2"));

    expect(actionRender).toHaveBeenCalledTimes(1);
    expect(connectionRender).toHaveBeenCalledTimes(2);
    screen.getByRole("button", { name: "delete" }).click();
    expect(onDeleteNode).toHaveBeenCalledExactlyOnceWith("node-1");
  });
});
