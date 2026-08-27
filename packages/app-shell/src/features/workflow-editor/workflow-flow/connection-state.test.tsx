import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { WorkflowConnectionStateProvider } from "./connection-state";
import { useWorkflowConnectionState } from "./use-connection-state";

/** Exposes the custom whole-card connection candidate for a semantic assertion. */
function ConnectionConsumer() {
  const { connectionCandidateEndpoint, connectionCandidateNodeId } =
    useWorkflowConnectionState();
  return (
    <span>
      {connectionCandidateNodeId ?? "none"}:
      {connectionCandidateEndpoint ?? "none"}
    </span>
  );
}

describe("WorkflowConnectionStateProvider", () => {
  it("provides the current whole-card connection candidate", () => {
    render(
      <WorkflowConnectionStateProvider
        value={{
          connectionCandidateEndpoint: "target",
          connectionCandidateNodeId: "node-2",
        }}
      >
        <ConnectionConsumer />
      </WorkflowConnectionStateProvider>,
    );

    expect(screen.getByText("node-2:target")).toBeInTheDocument();
  });
});
