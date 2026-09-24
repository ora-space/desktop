import { describe, expect, it } from "vitest";
import { isNodeWorking, runStatusTone } from "./run-status-style";

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

describe("retry_waiting status chrome", () => {
  it("uses the orange tone and its own label", () => {
    expect(runStatusTone("retry_waiting")).toEqual({
      dot: "bg-orange-500",
      ring: "border-orange-500/45 ring-orange-500/15",
      badge:
        "border-orange-500/30 bg-orange-500/10 text-orange-800 dark:text-orange-300",
      labelKey: "workflowRun.status.retry_waiting",
    });
  });

  it("keeps the waiting tone apart from running, HITL and failure", () => {
    const waiting = runStatusTone("retry_waiting");
    for (const other of ["running", "awaiting_input", "failed"] as const) {
      expect(runStatusTone(other).dot).not.toBe(waiting.dot);
      expect(runStatusTone(other).labelKey).not.toBe(waiting.labelKey);
    }
  });

  it("does not count a waiting node as working", () => {
    expect(isNodeWorking("retry_waiting")).toBe(false);
    expect(isNodeWorking("running")).toBe(true);
    expect(isNodeWorking("awaiting_input")).toBe(true);
  });
});
