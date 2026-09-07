import { describe, expect, it } from "vitest";
import { deriveWorkflowVariableCatalog, type WorkflowNodeData } from "../src";
import type { Edge, Node } from "@xyflow/react";

/** Builds a minimal workflow node for catalog projection tests. */
function node(
  id: string,
  data: WorkflowNodeData,
): Node<WorkflowNodeData, "workflow"> {
  return { id, type: "workflow", position: { x: 0, y: 0 }, data };
}

describe("deriveWorkflowVariableCatalog", () => {
  it("exposes variables from every upstream ancestor, including Start inputs", () => {
    const nodes = [
      node("start", {
        kind: "start",
        title: "开始",
        description: "",
        inputVariables: [{ name: "limit", valueType: "integer" }],
      }),
      node("writer", {
        kind: "agent",
        title: "Writer",
        description: "",
        agentConfig: {
          executor: { agentCli: "ora-space.codeagentcli", modelId: "gpt-5" },
          skills: [],
          prompt: "",
          interactive: false,
          outputContract: {
            type: "structured",
            schema: {
              type: "object",
              properties: { score: { type: "number" } },
            },
          },
        },
      }),
      node("condition", {
        kind: "condition",
        title: "条件",
        description: "",
      }),
      node("reviewer", {
        kind: "agent",
        title: "Reviewer",
        description: "",
        agentConfig: {
          executor: { agentCli: "ora-space.codeagentcli", modelId: "gpt-5" },
          skills: [],
          prompt: "",
          interactive: false,
        },
      }),
      node("indirect", {
        kind: "agent",
        title: "Indirect",
        description: "",
        agentConfig: {
          executor: { agentCli: "ora-space.codeagentcli", modelId: "gpt-5" },
          skills: [],
          prompt: "",
          interactive: false,
        },
      }),
      node("unrelated", {
        kind: "agent",
        title: "Unrelated",
        description: "",
        agentConfig: {
          executor: { agentCli: "ora-space.codeagentcli", modelId: "gpt-5" },
          skills: [],
          prompt: "",
          interactive: false,
        },
      }),
    ];
    const edges: Edge[] = [
      { id: "e1", source: "start", target: "indirect" },
      { id: "e2", source: "indirect", target: "writer" },
      { id: "e3", source: "writer", target: "condition" },
      { id: "e4", source: "reviewer", target: "condition" },
    ];

    expect(deriveWorkflowVariableCatalog(nodes, edges, "condition")).toEqual([
      {
        selector: ["sys", "workflow_id"],
        sourceNodeId: "sys",
        variableName: "workflow_id",
        valueType: "string",
      },
      {
        selector: ["sys", "timestamp"],
        sourceNodeId: "sys",
        variableName: "timestamp",
        valueType: "number",
      },
      {
        selector: ["start", "input"],
        sourceNodeId: "start",
        variableName: "input",
        valueType: "string",
      },
      {
        selector: ["start", "limit"],
        sourceNodeId: "start",
        variableName: "limit",
        valueType: "integer",
      },
      {
        selector: ["writer", "output"],
        sourceNodeId: "writer",
        variableName: "output",
        valueType: "string",
      },
      {
        selector: ["writer", "structured_output"],
        sourceNodeId: "writer",
        variableName: "structured_output",
        valueType: "object",
      },
      {
        selector: ["writer", "structured_output", "score"],
        sourceNodeId: "writer",
        variableName: "structured_output.score",
        valueType: "number",
      },
      {
        selector: ["reviewer", "output"],
        sourceNodeId: "reviewer",
        variableName: "output",
        valueType: "string",
      },
      {
        selector: ["indirect", "output"],
        sourceNodeId: "indirect",
        variableName: "output",
        valueType: "string",
      },
    ]);

    expect(deriveWorkflowVariableCatalog(nodes, edges, "indirect")).toEqual([
      {
        selector: ["sys", "workflow_id"],
        sourceNodeId: "sys",
        variableName: "workflow_id",
        valueType: "string",
      },
      {
        selector: ["sys", "timestamp"],
        sourceNodeId: "sys",
        variableName: "timestamp",
        valueType: "number",
      },
      {
        selector: ["start", "input"],
        sourceNodeId: "start",
        variableName: "input",
        valueType: "string",
      },
      {
        selector: ["start", "limit"],
        sourceNodeId: "start",
        variableName: "limit",
        valueType: "integer",
      },
    ]);
  });

  it("does not expose output or internal routing state from a Condition", () => {
    const nodes = [
      node("start", {
        kind: "start",
        title: "开始",
        description: "",
        inputVariables: [
          { name: "output", valueType: "string", value: "request" },
        ],
      }),
      node("condition-0", {
        kind: "condition",
        title: "前置条件",
        description: "",
      }),
      node("condition-1", {
        kind: "condition",
        title: "条件分支",
        description: "",
      }),
      node("agent-1", {
        kind: "agent",
        title: "Agent",
        description: "",
        agentConfig: {
          executor: { agentCli: "ora-space.codeagentcli", modelId: "gpt-5" },
          skills: [],
          prompt: "",
          interactive: false,
        },
      }),
    ];
    const edges: Edge[] = [
      { id: "start-condition", source: "start", target: "condition-0" },
      {
        id: "condition-chain",
        source: "condition-0",
        target: "condition-1",
      },
      { id: "condition-agent", source: "condition-1", target: "agent-1" },
    ];

    expect(deriveWorkflowVariableCatalog(nodes, edges, "agent-1")).toEqual([
      {
        selector: ["sys", "workflow_id"],
        sourceNodeId: "sys",
        variableName: "workflow_id",
        valueType: "string",
      },
      {
        selector: ["sys", "timestamp"],
        sourceNodeId: "sys",
        variableName: "timestamp",
        valueType: "number",
      },
      {
        selector: ["start", "input"],
        sourceNodeId: "start",
        variableName: "input",
        valueType: "string",
      },
      {
        selector: ["start", "output"],
        sourceNodeId: "start",
        variableName: "output",
        valueType: "string",
      },
    ]);
  });

  it("keeps every ancestor visible on a branched path", () => {
    const agentConfig = {
      schemaVersion: 3 as const,
      executor: { agentCli: "ora-space.codeagentcli", modelId: "gpt-5" },
      roleId: "test",
      skills: [],
      mcps: [],
      prompt: "",
      interactive: false,
    };
    const nodes = [
      node("a", {
        kind: "start",
        title: "A",
        description: "",
        inputVariables: [{ name: "request", valueType: "string" }],
      }),
      node("b", { kind: "agent", title: "B", description: "", agentConfig }),
      node("c", { kind: "agent", title: "C", description: "", agentConfig }),
      node("d", {
        kind: "agent",
        title: "D",
        description: "",
        agentConfig: {
          ...agentConfig,
          outputContract: {
            type: "structured",
            schema: {
              type: "object",
              properties: { score: { type: "number" } },
            },
          },
        },
      }),
      node("e", { kind: "agent", title: "E", description: "", agentConfig }),
      node("f", { kind: "agent", title: "F", description: "", agentConfig }),
    ];
    const edges: Edge[] = [
      { id: "a-b", source: "a", target: "b" },
      { id: "b-c", source: "b", target: "c" },
      { id: "a-d", source: "a", target: "d" },
      { id: "d-e", source: "d", target: "e" },
      { id: "e-f", source: "e", target: "f" },
    ];

    expect(
      deriveWorkflowVariableCatalog(nodes, edges, "f").map(
        (variable) => variable.selector,
      ),
    ).toEqual([
      ["sys", "workflow_id"],
      ["sys", "timestamp"],
      ["a", "input"],
      ["a", "request"],
      ["d", "output"],
      ["d", "structured_output"],
      ["d", "structured_output", "score"],
      ["e", "output"],
    ]);
  });

  it("deduplicates producers that reach a consumer through multiple conditions", () => {
    const nodes = [
      node("writer", {
        kind: "agent",
        title: "Writer",
        description: "",
      }),
      node("condition-left", {
        kind: "condition",
        title: "Left",
        description: "",
      }),
      node("condition-right", {
        kind: "condition",
        title: "Right",
        description: "",
      }),
      node("reader", {
        kind: "agent",
        title: "Reader",
        description: "",
      }),
    ];
    const edges: Edge[] = [
      { id: "writer-left", source: "writer", target: "condition-left" },
      { id: "writer-right", source: "writer", target: "condition-right" },
      { id: "left-reader", source: "condition-left", target: "reader" },
      { id: "right-reader", source: "condition-right", target: "reader" },
    ];

    expect(
      deriveWorkflowVariableCatalog(nodes, edges, "reader").filter(
        (variable) => variable.selector[0] === "writer",
      ),
    ).toEqual([
      {
        selector: ["writer", "output"],
        sourceNodeId: "writer",
        variableName: "output",
        valueType: "string",
      },
    ]);
  });
});
