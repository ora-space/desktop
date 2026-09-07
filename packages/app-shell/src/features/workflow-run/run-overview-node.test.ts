import { describe, expect, it } from "vitest";
import type { WorkflowNodeData } from "@ora/workflow-runtime";
import { resolveRunOverviewSourceHandleIds } from "./run-overview-handles";

describe("resolveRunOverviewSourceHandleIds", () => {
  it("restores every condition branch handle used by frozen snapshot edges", () => {
    const condition: WorkflowNodeData = {
      kind: "condition",
      title: "Route",
      description: "",
      cases: [
        { id: "case-1", conditions: [] },
        { id: "case-2", conditions: [] },
      ],
    };

    expect(resolveRunOverviewSourceHandleIds(condition)).toEqual([
      "case-1",
      "case-2",
      "else",
    ]);
  });

  it("keeps the implicit source handle for ordinary nodes", () => {
    expect(
      resolveRunOverviewSourceHandleIds({
        kind: "agent",
        title: "Agent",
        description: "",
      }),
    ).toBeNull();
  });
});
