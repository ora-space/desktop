import { describe, expect, it } from "vitest";
import { projectNodeStatus, projectRunStatus } from "./run-projection";

describe("run status projection", () => {
  it("projects a not-started pending run as pending", () => {
    expect(projectRunStatus("pending", [])).toBe("pending");
  });

  it("derives awaiting_input from a pending run with waiting nodes", () => {
    expect(projectRunStatus("pending", ["prompt-1"])).toBe("awaiting_input");
  });

  it("maps the terminal backend states one-to-one", () => {
    expect(projectRunStatus("running", [])).toBe("running");
    expect(projectRunStatus("succeeded", [])).toBe("succeeded");
    expect(projectRunStatus("failed", ["explore"])).toBe("failed");
    expect(projectRunStatus("cancelled", [])).toBe("cancelled");
  });
});

describe("node status projection", () => {
  it("projects a graph node with no node-run row as idle", () => {
    expect(projectNodeStatus(null)).toBe("idle");
  });

  it("derives awaiting_input from a pending node-run", () => {
    expect(projectNodeStatus({ status: "pending" })).toBe("awaiting_input");
  });

  it("maps the remaining node-run states one-to-one", () => {
    expect(projectNodeStatus({ status: "running" })).toBe("running");
    expect(projectNodeStatus({ status: "succeeded" })).toBe("succeeded");
    expect(projectNodeStatus({ status: "failed" })).toBe("failed");
    expect(projectNodeStatus({ status: "cancelled" })).toBe("cancelled");
  });
});
