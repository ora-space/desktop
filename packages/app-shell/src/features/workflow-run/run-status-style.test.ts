import { describe, expect, it } from "vitest";
import { isTerminalRunStatus, runStatusTone } from "./run-status-style";

describe("isTerminalRunStatus", () => {
  it("marks finished run statuses only", () => {
    expect(isTerminalRunStatus("succeeded")).toBe(true);
    expect(isTerminalRunStatus("failed")).toBe(true);
    expect(isTerminalRunStatus("cancelled")).toBe(true);
    expect(isTerminalRunStatus("running")).toBe(false);
    expect(isTerminalRunStatus("awaiting_input")).toBe(false);
    expect(isTerminalRunStatus("pending")).toBe(false);
  });
});

describe("runStatusTone", () => {
  it("maps terminal outcomes to distinct label keys", () => {
    expect(runStatusTone("succeeded").labelKey).toBe("workflowRun.status.succeeded");
    expect(runStatusTone("failed").labelKey).toBe("workflowRun.status.failed");
    expect(runStatusTone("cancelled").labelKey).toBe("workflowRun.status.cancelled");
  });

  it("keeps awaiting_input in the amber HITL family", () => {
    expect(runStatusTone("awaiting_input").dot).toContain("amber");
    expect(runStatusTone("succeeded").dot).toContain("emerald");
    expect(runStatusTone("failed").dot).toContain("rose");
  });
});
