import { describe, expect, it } from "vitest";
import {
  WORKFLOW_AGENT_COMPLETION_MAX_FILES,
  WORKFLOW_AGENT_COMPLETION_MAX_PATH_BYTES,
  isWorkflowAgentCompletion,
  workflowAgentCompletionIssues,
  workflowAgentCompletionPathIssue,
} from "../src/agent-completion";
import { createMockWorkflow } from "../src/fixtures";
import { normalizeWorkflowAgentConfig } from "../src/normalize-agent-config";
import type { WorkflowAgentConfig } from "../src/node-data";
import { isDemoWorkflow } from "../src/validation";

/** Rebuilds the mock workflow around one Agent configuration under test. */
function workflowWithAgentConfig(config: unknown) {
  const workflow = createMockWorkflow("en-US");
  const nodes = workflow.nodes.map((node) =>
    node.data.kind === "agent"
      ? {
          ...node,
          data: {
            ...node.data,
            agentConfig: config as WorkflowAgentConfig,
          },
        }
      : node,
  );
  return { ...workflow, nodes };
}

/** Reads the first Agent node configuration from a workflow fixture. */
function firstAgentConfig(): WorkflowAgentConfig {
  const node = createMockWorkflow("en-US").nodes.find(
    (candidate) => candidate.data.kind === "agent",
  );
  if (node === undefined) {
    throw new Error("fixture has an Agent node");
  }
  return node.data.agentConfig!;
}

describe("agent completion declarations", () => {
  it("accepts the supported wildcards and an omitted mustChange", () => {
    expect(
      workflowAgentCompletionIssues({
        files: [
          { path: "reports/*.md" },
          { path: "reports/**/final.md", mustChange: true },
          { path: "reports/?.md" },
        ],
      }),
    ).toEqual([]);
  });

  it("refuses unsupported fields instead of dropping them", () => {
    expect(
      workflowAgentCompletionIssues({
        files: [{ path: "reports/plan.md" }],
        criteria: ["the plan is complete"],
      }),
    ).not.toEqual([]);
    expect(
      workflowAgentCompletionIssues({
        files: [{ path: "reports/plan.md" }],
        checklist: [],
      }),
    ).not.toEqual([]);
    expect(
      workflowAgentCompletionIssues({
        files: [{ path: "reports/plan.md", criteria: [] }],
      }),
    ).not.toEqual([]);
  });

  it("bounds the declaration count, path bytes, and path syntax", () => {
    expect(workflowAgentCompletionIssues({ files: [] })).not.toEqual([]);
    expect(
      workflowAgentCompletionIssues({
        files: Array.from(
          { length: WORKFLOW_AGENT_COMPLETION_MAX_FILES + 1 },
          (_, index) => ({ path: `file-${index}.md` }),
        ),
      }),
    ).not.toEqual([]);
    expect(
      workflowAgentCompletionPathIssue(
        "x".repeat(WORKFLOW_AGENT_COMPLETION_MAX_PATH_BYTES + 1),
      ),
    ).not.toBeNull();
    // Length is measured in UTF-8 bytes, not characters.
    expect(
      workflowAgentCompletionPathIssue(
        "名".repeat(
          Math.floor(WORKFLOW_AGENT_COMPLETION_MAX_PATH_BYTES / 3) + 1,
        ),
      ),
    ).not.toBeNull();
    for (const path of [
      "/absolute.md",
      "../outside.md",
      "a//b.md",
      "a/./b.md",
      "a\\b.md",
      "a/[0-9].md",
      "a/{one,two}.md",
      "!reports/*.md",
      "a/**b/c.md",
      "a/***/c.md",
      "CON",
      "reports/C:/plan.md",
      `${"a".repeat(256)}/file.md`,
      Array.from({ length: 34 }, (_, index) => `d${index}`).join("/"),
    ]) {
      expect(workflowAgentCompletionPathIssue(path), path).not.toBeNull();
    }
  });

  it("rejects an explicit null mustChange while accepting an omitted one", () => {
    expect(
      workflowAgentCompletionIssues({
        files: [{ path: "reports/plan.md", mustChange: null }],
      }),
    ).not.toEqual([]);
    expect(
      workflowAgentCompletionIssues({
        files: [{ path: "reports/plan.md" }],
      }),
    ).toEqual([]);
  });

  it("keeps the declared-completion type guard false for an absent declaration", () => {
    expect(isWorkflowAgentCompletion(undefined)).toBe(false);
    expect(isWorkflowAgentCompletion(null)).toBe(false);
    expect(isWorkflowAgentCompletion({ files: [{ path: "a.md" }] })).toBe(true);
  });

  it("validates the version 3/4 matrix on imported graphs", () => {
    const base = firstAgentConfig();
    expect(
      isDemoWorkflow(workflowWithAgentConfig({ ...base, schemaVersion: 3 })),
    ).toBe(true);
    expect(
      isDemoWorkflow(workflowWithAgentConfig({ ...base, schemaVersion: 4 })),
    ).toBe(true);
    expect(
      isDemoWorkflow(
        workflowWithAgentConfig({
          ...base,
          schemaVersion: 4,
          completion: { files: [{ path: "reports/plan.md" }] },
        }),
      ),
    ).toBe(true);
    expect(
      isDemoWorkflow(
        workflowWithAgentConfig({
          ...base,
          schemaVersion: 3,
          completion: { files: [{ path: "reports/plan.md" }] },
        }),
      ),
    ).toBe(false);
    expect(
      isDemoWorkflow(workflowWithAgentConfig({ ...base, schemaVersion: 5 })),
    ).toBe(false);
  });

  it("normalizes completion explicitly and keeps the declared version", () => {
    const config: WorkflowAgentConfig = {
      ...firstAgentConfig(),
      schemaVersion: 4,
      completion: {
        files: [
          { path: "reports/*.md" },
          { path: "reports/final.md", mustChange: true },
        ],
      },
    };
    const normalized = normalizeWorkflowAgentConfig(config);
    expect(normalized.schemaVersion).toBe(4);
    expect(normalized.completion).toEqual({
      files: [
        { path: "reports/*.md" },
        { path: "reports/final.md", mustChange: true },
      ],
    });

    // A legacy config without completion stays without the field.
    const legacy = normalizeWorkflowAgentConfig(firstAgentConfig());
    expect(legacy).not.toHaveProperty("completion");
  });

  it("does not throw when a malformed completion is normalized for display", () => {
    const config = {
      ...firstAgentConfig(),
      completion: { criteria: [] },
    } as unknown as WorkflowAgentConfig;
    const normalized = normalizeWorkflowAgentConfig(config);
    expect(normalized.completion).toEqual({ criteria: [] });
    expect(isDemoWorkflow(workflowWithAgentConfig(config))).toBe(false);
  });
});
