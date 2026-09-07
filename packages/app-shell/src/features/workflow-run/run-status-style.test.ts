import { describe, expect, it } from "vitest";
import { runStatusTone } from "./run-status-style";

describe("runStatusTone", () => {
  it("maps terminal outcomes to distinct label keys", () => {
    expect(runStatusTone("succeeded").labelKey).toBe(
      "workflowRun.status.succeeded",
    );
    expect(runStatusTone("failed").labelKey).toBe("workflowRun.status.failed");
    expect(runStatusTone("cancelled").labelKey).toBe(
      "workflowRun.status.cancelled",
    );
  });

  it("keeps awaiting_input in the amber HITL family", () => {
    expect(runStatusTone("awaiting_input").dot).toContain("amber");
    expect(runStatusTone("succeeded").dot).toContain("emerald");
    expect(runStatusTone("failed").dot).toContain("rose");
  });

  it("labels inactive branch nodes distinctly from idle", () => {
    expect(runStatusTone("inactive").labelKey).toBe(
      "workflowRun.nodeStatus.inactive",
    );
    expect(runStatusTone("inactive").labelKey).not.toBe(
      runStatusTone("idle").labelKey,
    );
  });
});
