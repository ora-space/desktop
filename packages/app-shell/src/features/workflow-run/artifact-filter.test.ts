import { describe, expect, it } from "vitest";
import { filterArtifacts, latestArtifact } from "./artifact-filter";
import type { WorkflowArtifact } from "@ora/workflow-runtime";

function artifact(
  partial: Pick<WorkflowArtifact, "id" | "nodeId" | "createdAt">,
): WorkflowArtifact {
  return {
    runId: "gwr-1",
    kind: "markdown",
    title: partial.id,
    body: "body",
    ...partial,
  };
}

describe("filterArtifacts", () => {
  const items = [
    artifact({ id: "a", nodeId: "n1", createdAt: "2026-08-01T12:00:01+08:00" }),
    artifact({ id: "b", nodeId: "n2", createdAt: "2026-08-01T12:00:03+08:00" }),
    artifact({ id: "c", nodeId: "n1", createdAt: "2026-08-01T12:00:02+08:00" }),
  ];

  it("returns newest-first for the full run", () => {
    expect(filterArtifacts(items, { type: "all" }).map((item) => item.id)).toEqual([
      "b",
      "c",
      "a",
    ]);
  });

  it("scopes to one node and keeps newest-first", () => {
    expect(
      filterArtifacts(items, { type: "node", nodeId: "n1" }).map((item) => item.id),
    ).toEqual(["c", "a"]);
  });

  it("returns an empty list when the node has no artifacts", () => {
    expect(filterArtifacts(items, { type: "node", nodeId: "missing" })).toEqual([]);
  });
});

describe("latestArtifact", () => {
  it("returns null for an empty list", () => {
    expect(latestArtifact([])).toBeNull();
  });

  it("returns the newest artifact", () => {
    expect(
      latestArtifact([
        artifact({ id: "a", nodeId: "n1", createdAt: "2026-08-01T12:00:01+08:00" }),
        artifact({ id: "b", nodeId: "n2", createdAt: "2026-08-01T12:00:03+08:00" }),
        artifact({ id: "c", nodeId: "n1", createdAt: "2026-08-01T12:00:02+08:00" }),
      ]),
    ).toEqual(
      expect.objectContaining({ id: "b", nodeId: "n2" }),
    );
  });
});
