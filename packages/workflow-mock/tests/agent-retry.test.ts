import { describe, expect, it } from "vitest";
import {
  DEFAULT_WORKFLOW_AGENT_RETRY,
  WORKFLOW_AGENT_RETRY_BOUNDS,
  WORKFLOW_AGENT_RETRY_MAX_DELAY_SECONDS,
  createMockWorkflow,
  parseDemoWorkflow,
  resolveWorkflowAgentRetryPolicy,
  validateWorkflowAgentRetry,
  workflowAgentRetryApplies,
  type WorkflowAgentRetryPolicy,
} from "../src";

describe("agent retry defaults and bounds", () => {
  it("matches the engine contract for an absent retry field", () => {
    expect(DEFAULT_WORKFLOW_AGENT_RETRY).toEqual({
      enabled: true,
      maxRetries: 2,
      initialDelaySeconds: 10,
    });
    expect(WORKFLOW_AGENT_RETRY_BOUNDS).toEqual({
      maxRetries: { min: 0, max: 5 },
      initialDelaySeconds: { min: 0, max: 300 },
    });
    expect(WORKFLOW_AGENT_RETRY_MAX_DELAY_SECONDS).toBe(600);
  });

  it("cannot be mutated by consumers", () => {
    expect(Object.isFrozen(DEFAULT_WORKFLOW_AGENT_RETRY)).toBe(true);
    expect(Object.isFrozen(WORKFLOW_AGENT_RETRY_BOUNDS.maxRetries)).toBe(true);
    expect(
      Object.isFrozen(WORKFLOW_AGENT_RETRY_BOUNDS.initialDelaySeconds),
    ).toBe(true);
  });

  it("keeps the default inside its own bounds", () => {
    expect(validateWorkflowAgentRetry(DEFAULT_WORKFLOW_AGENT_RETRY)).toEqual(
      [],
    );
  });
});

describe("resolveWorkflowAgentRetryPolicy", () => {
  it("returns a fresh copy of the default when retry is absent", () => {
    const resolved = resolveWorkflowAgentRetryPolicy({});
    expect(resolved).toEqual({
      enabled: true,
      maxRetries: 2,
      initialDelaySeconds: 10,
    });
    expect(resolved).not.toBe(DEFAULT_WORKFLOW_AGENT_RETRY);
    resolved.maxRetries = 5;
    expect(DEFAULT_WORKFLOW_AGENT_RETRY.maxRetries).toBe(2);
  });

  it("treats null like absent, matching the backend decoder", () => {
    expect(resolveWorkflowAgentRetryPolicy({ retry: null })).toEqual(
      DEFAULT_WORKFLOW_AGENT_RETRY,
    );
  });

  it("returns stored values, including a turned-off policy", () => {
    const stored: WorkflowAgentRetryPolicy = {
      enabled: false,
      maxRetries: 4,
      initialDelaySeconds: 0,
    };
    const resolved = resolveWorkflowAgentRetryPolicy({ retry: stored });
    expect(resolved).toEqual(stored);
    expect(resolved).not.toBe(stored);
  });

  it("returns only the three policy fields", () => {
    const stored = {
      enabled: true,
      maxRetries: 1,
      initialDelaySeconds: 30,
      futureField: "kept out",
    } as WorkflowAgentRetryPolicy;
    expect(resolveWorkflowAgentRetryPolicy({ retry: stored })).toEqual({
      enabled: true,
      maxRetries: 1,
      initialDelaySeconds: 30,
    });
  });
});

describe("workflowAgentRetryApplies", () => {
  it("never applies to interactive nodes", () => {
    expect(workflowAgentRetryApplies({})).toBe(true);
    expect(workflowAgentRetryApplies({ interactive: false })).toBe(true);
    expect(workflowAgentRetryApplies({ interactive: true })).toBe(false);
  });
});

describe("validateWorkflowAgentRetry", () => {
  it("accepts an absent value because it means the default", () => {
    expect(validateWorkflowAgentRetry(undefined)).toEqual([]);
    expect(validateWorkflowAgentRetry(null)).toEqual([]);
  });

  it.each([
    { enabled: true, maxRetries: 0, initialDelaySeconds: 0 },
    { enabled: true, maxRetries: 5, initialDelaySeconds: 300 },
    { enabled: false, maxRetries: 0, initialDelaySeconds: 300 },
    { enabled: false, maxRetries: 5, initialDelaySeconds: 0 },
  ])("accepts the inclusive boundaries %o", (policy) => {
    expect(validateWorkflowAgentRetry(policy)).toEqual([]);
  });

  it.each([
    ["maxRetries", -1],
    ["maxRetries", 6],
    ["initialDelaySeconds", -1],
    ["initialDelaySeconds", 301],
  ] as const)("rejects %s = %d as out of range", (field, value) => {
    expect(
      validateWorkflowAgentRetry({
        ...DEFAULT_WORKFLOW_AGENT_RETRY,
        [field]: value,
      }),
    ).toEqual([{ field, reason: "outOfRange" }]);
  });

  it.each([
    ["maxRetries", 1.5],
    ["initialDelaySeconds", 1.5],
    ["maxRetries", "3"],
    ["initialDelaySeconds", "10"],
    ["maxRetries", Number.NaN],
    ["initialDelaySeconds", Number.POSITIVE_INFINITY],
    ["maxRetries", true],
  ] as const)("rejects %s = %o as not an integer", (field, value) => {
    expect(
      validateWorkflowAgentRetry({
        ...DEFAULT_WORKFLOW_AGENT_RETRY,
        [field]: value,
      }),
    ).toEqual([{ field, reason: "notInteger" }]);
  });

  it("requires every sub-field once the object is present", () => {
    expect(validateWorkflowAgentRetry({})).toEqual([
      { field: "enabled", reason: "missing" },
      { field: "maxRetries", reason: "missing" },
      { field: "initialDelaySeconds", reason: "missing" },
    ]);
    expect(
      validateWorkflowAgentRetry({ enabled: true, maxRetries: 2 }),
    ).toEqual([{ field: "initialDelaySeconds", reason: "missing" }]);
    expect(
      validateWorkflowAgentRetry({ maxRetries: 2, initialDelaySeconds: 10 }),
    ).toEqual([{ field: "enabled", reason: "missing" }]);
    expect(
      validateWorkflowAgentRetry({
        enabled: true,
        maxRetries: null,
        initialDelaySeconds: 10,
      }),
    ).toEqual([{ field: "maxRetries", reason: "missing" }]);
  });

  it("rejects a non-boolean switch", () => {
    expect(
      validateWorkflowAgentRetry({
        ...DEFAULT_WORKFLOW_AGENT_RETRY,
        enabled: "yes",
      }),
    ).toEqual([{ field: "enabled", reason: "notBoolean" }]);
  });

  it.each(["retry", 3, true, [], [1, 2, 3]])(
    "rejects the non-object %o",
    (value) => {
      expect(validateWorkflowAgentRetry(value)).toEqual([
        { field: null, reason: "notObject" },
      ]);
    },
  );

  it("reports every problem at once in field order", () => {
    expect(
      validateWorkflowAgentRetry({
        enabled: 1,
        maxRetries: 9,
        initialDelaySeconds: 0.5,
      }),
    ).toEqual([
      { field: "enabled", reason: "notBoolean" },
      { field: "maxRetries", reason: "outOfRange" },
      { field: "initialDelaySeconds", reason: "notInteger" },
    ]);
  });
});

describe("demo workflow validation of retry", () => {
  /** The code review fixture with its first Agent node's `retry` set to the given raw value. */
  function workflowWithRetry(retry: unknown) {
    const workflow = createMockWorkflow("en-US");
    const agent = workflow.nodes.find((node) => node.data.kind === "agent");
    if (agent?.data.agentConfig === undefined) {
      throw new Error("The code review fixture requires an Agent node");
    }
    agent.data.agentConfig = {
      ...agent.data.agentConfig,
      retry: retry as WorkflowAgentRetryPolicy,
    };
    return workflow;
  }

  it("accepts absent and valid policies", () => {
    const withoutRetry = createMockWorkflow("en-US");
    expect(parseDemoWorkflow(withoutRetry)).toEqual(withoutRetry);
    const valid = workflowWithRetry({
      enabled: false,
      maxRetries: 5,
      initialDelaySeconds: 300,
    });
    expect(parseDemoWorkflow(valid)).toEqual(valid);
  });

  it.each([
    { enabled: true, maxRetries: 6, initialDelaySeconds: 10 },
    { enabled: true, maxRetries: 2, initialDelaySeconds: 301 },
    { enabled: true, maxRetries: 1.5, initialDelaySeconds: 10 },
    { enabled: true, maxRetries: 2 },
    "yes",
  ])("rejects the invalid policy %o", (retry) => {
    expect(() => parseDemoWorkflow(workflowWithRetry(retry))).toThrow(
      "Invalid workflow definition",
    );
  });
});
