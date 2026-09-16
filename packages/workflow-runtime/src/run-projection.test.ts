import { describe, expect, it } from "vitest";
import {
  isTerminalRunStatus,
  projectNodeStatus,
  projectRunStatus,
  toDisplayRunStatus,
  toListRunStatus,
} from "./run-projection";

describe("run status projection", () => {
  it("projects a not-started pending run as pending", () => {
    expect(projectRunStatus("pending", [])).toBe("pending");
  });

  it("derives awaiting_input from a pending run with waiting nodes", () => {
    expect(projectRunStatus("pending", ["prompt-1"])).toBe("awaiting_input");
  });

  it("maps the awaitingInput wire status onto awaiting_input", () => {
    expect(projectRunStatus("awaitingInput", [])).toBe("awaiting_input");
  });

  it("maps the terminal backend states one-to-one", () => {
    expect(projectRunStatus("running", [])).toBe("running");
    expect(projectRunStatus("succeeded", [])).toBe("succeeded");
    expect(projectRunStatus("failed", ["explore"])).toBe("failed");
    expect(projectRunStatus("cancelled", [])).toBe("cancelled");
  });
});

describe("list and display status round-trip", () => {
  it("keeps sidebar and Theater on the same display status", () => {
    expect(toDisplayRunStatus("awaitingInput")).toBe("awaiting_input");
    expect(toListRunStatus("awaiting_input")).toBe("awaitingInput");
    expect(toDisplayRunStatus("running")).toBe("running");
    expect(toListRunStatus("succeeded")).toBe("succeeded");
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

describe("toDisplayRunStatus", () => {
  it("maps awaitingInput onto the display spelling used by Theater", () => {
    expect(toDisplayRunStatus("awaitingInput")).toBe("awaiting_input");
  });

  it("keeps every other wire status unchanged", () => {
    expect(toDisplayRunStatus("pending")).toBe("pending");
    expect(toDisplayRunStatus("running")).toBe("running");
    expect(toDisplayRunStatus("succeeded")).toBe("succeeded");
    expect(toDisplayRunStatus("failed")).toBe("failed");
    expect(toDisplayRunStatus("cancelled")).toBe("cancelled");
  });
});

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
