import { describe, expect, it } from "vitest";
import {
  createDemoWorkflow,
  createMockWorkflows,
  createMockWorkflow,
  parseDemoWorkflow,
} from "../src";

describe("workflow demo", () => {
  it("creates a usable session graph with exactly one start node", () => {
    const workflow = createDemoWorkflow("demo-1", "Demo", "en-US");

    expect(workflow).toEqual({
      id: "demo-1",
      name: "Demo",
      description: "No description yet",
      updatedAt: workflow.updatedAt,
      viewport: { x: 32, y: 32, zoom: 1 },
      nodes: [
        {
          id: "start",
          type: "workflow",
          deletable: false,
          position: { x: 120, y: 260 },
          data: {
            kind: "start",
            title: "Start",
            description: "Receive workflow input",
            instruction: "Define the input required to start this workflow.",
          },
        },
      ],
      edges: [],
    });
  });

  it("returns an isolated imported definition", () => {
    const source = createMockWorkflow("en-US");
    source.viewport = { x: -120, y: 48, zoom: 0.75 };
    const imported = parseDemoWorkflow(source);

    imported.nodes[0]!.data.title = "Changed";

    expect(source.nodes[0]!.data.title).toBe("Start");
    expect(imported.viewport).toEqual({ x: -120, y: 48, zoom: 0.75 });
  });

  it("round-trips the native React Flow snapshot through JSON storage", () => {
    const source = createMockWorkflow("en-US");
    source.nodes[1]!.selected = true;
    source.edges[0]!.selected = true;
    source.viewport = { x: -84, y: 26, zoom: 1.25 };

    const restored = parseDemoWorkflow(JSON.parse(JSON.stringify(source)));

    expect(restored).toEqual(source);
  });

  it("uses only React Flow's dynamic initial measurements for demo fixtures", () => {
    const [node] = createMockWorkflow("en-US").nodes;

    expect(node).toMatchObject({
      initialWidth: 230,
      initialHeight: 98,
      handles: [
        {
          type: "target",
          position: "left",
          x: -5,
          y: 56,
          width: 10,
          height: 10,
        },
        {
          type: "source",
          position: "right",
          x: 225,
          y: 56,
          width: 10,
          height: 10,
        },
      ],
    });
    expect(node).not.toHaveProperty("width");
    expect(node).not.toHaveProperty("height");
    expect(node).not.toHaveProperty("style");
  });

  it("rejects malformed imports", () => {
    expect(() => parseDemoWorkflow({ nodes: [], edges: [] })).toThrow(
      "Invalid workflow definition",
    );

    const deletableStart = createMockWorkflow("en-US");
    deletableStart.nodes[0]!.deletable = true;
    expect(() => parseDemoWorkflow(deletableStart)).toThrow(
      "Invalid workflow definition",
    );

    const unsupportedEdge = createMockWorkflow("en-US");
    unsupportedEdge.edges[0]!.type = "unknown";
    expect(() => parseDemoWorkflow(unsupportedEdge)).toThrow(
      "Invalid workflow definition",
    );

    const missingHandle = createMockWorkflow("en-US");
    missingHandle.edges[0]!.sourceHandle = "missing";
    expect(() => parseDemoWorkflow(missingHandle)).toThrow(
      "Invalid workflow definition",
    );

    const missingAgentContract = createMockWorkflow("en-US");
    const agent = missingAgentContract.nodes.find(
      (node) => node.data.kind === "agent",
    );
    if (agent === undefined) {
      throw new Error("The code review fixture requires an Agent node");
    }
    delete agent.data.agentConfig;
    expect(() => parseDemoWorkflow(missingAgentContract)).toThrow(
      "Invalid workflow definition",
    );
  });

  it("accepts editor annotations and rejects identifiers shared with executable nodes", () => {
    const workflow = createMockWorkflow("en-US");
    workflow.annotations = [
      {
        id: "annotation-1",
        type: "annotation",
        position: { x: 40, y: 60 },
        width: 240,
        height: 140,
        data: { text: "Review this branch", theme: "yellow" },
      },
    ];

    expect(parseDemoWorkflow(workflow)).toEqual(workflow);

    workflow.annotations[0]!.id = workflow.nodes[0]!.id;
    expect(() => parseDemoWorkflow(workflow)).toThrow(
      "Invalid workflow definition",
    );
  });

  it("includes a seven-stage Agent lifecycle demo with explicit execution contracts", () => {
    const workflow = createMockWorkflows("en-US").find(
      (candidate) => candidate.id === "spec-change-lifecycle",
    );

    expect(workflow).toMatchObject({
      id: "spec-change-lifecycle",
      name: "OpenSpec workflow demo",
      nodes: [
        expect.objectContaining({
          id: "start",
          data: expect.objectContaining({ kind: "start" }),
        }),
        expect.objectContaining({
          id: "explore",
          data: expect.objectContaining({
            kind: "agent",
            title: "Explore",
            agentConfig: expect.objectContaining({
              roleId: "Researcher",
            }),
          }),
        }),
        expect.objectContaining({
          id: "sfmea-review",
          data: expect.objectContaining({
            kind: "agent",
            title: "SFMEA review",
            agentConfig: expect.objectContaining({
              roleId: "Reviewer",
              skills: [
                expect.objectContaining({ skillId: "cdase:sfmea_review" }),
              ],
            }),
          }),
        }),
        expect.objectContaining({
          id: "propose",
          data: expect.objectContaining({
            kind: "agent",
            title: "Propose",
            agentConfig: expect.objectContaining({ roleId: "Planner" }),
          }),
        }),
        expect.objectContaining({
          id: "apply",
          data: expect.objectContaining({
            kind: "agent",
            title: "Apply",
            agentConfig: expect.objectContaining({
              roleId: "Implementer",
            }),
          }),
        }),
        expect.objectContaining({
          id: "code-defect-scan",
          data: expect.objectContaining({
            kind: "agent",
            title: "Code defect scan",
            agentConfig: expect.objectContaining({
              roleId: "Reviewer",
              skills: [
                expect.objectContaining({ skillId: "code-defect-scan" }),
              ],
            }),
          }),
        }),
        expect.objectContaining({
          id: "defect-repair",
          data: expect.objectContaining({
            kind: "agent",
            title: "Defect repair",
            agentConfig: expect.objectContaining({
              roleId: "Implementer",
              skills: [],
            }),
          }),
        }),
        expect.objectContaining({
          id: "archive",
          data: expect.objectContaining({
            kind: "agent",
            title: "Archive",
            agentConfig: expect.objectContaining({
              roleId: "Documentation Agent",
            }),
          }),
        }),
      ],
      edges: [
        expect.objectContaining({ source: "start", target: "explore" }),
        expect.objectContaining({ source: "explore", target: "sfmea-review" }),
        expect.objectContaining({ source: "sfmea-review", target: "propose" }),
        expect.objectContaining({ source: "propose", target: "apply" }),
        expect.objectContaining({
          source: "apply",
          target: "code-defect-scan",
        }),
        expect.objectContaining({
          source: "code-defect-scan",
          target: "defect-repair",
        }),
        expect.objectContaining({ source: "defect-repair", target: "archive" }),
      ],
    });
    expect(parseDemoWorkflow(workflow)).toEqual(workflow);
    expect(
      workflow.nodes
        .filter((node) => node.data.kind === "agent")
        .map((node) => node.data.agentConfig?.executor),
    ).toEqual([
      { agentCli: "ora-space.opencode", modelId: "deepseek/deepseek-v4-pro" },
      { agentCli: "ora-space.opencode", modelId: "deepseek/deepseek-v4-pro" },
      { agentCli: "ora-space.opencode", modelId: "deepseek/deepseek-v4-pro" },
      { agentCli: "ora-space.opencode", modelId: "deepseek/deepseek-v4-flash" },
      { agentCli: "ora-space.opencode", modelId: "deepseek/deepseek-v4-pro" },
      { agentCli: "ora-space.opencode", modelId: "deepseek/deepseek-v4-flash" },
      { agentCli: "ora-space.opencode", modelId: "deepseek/deepseek-v4-flash" },
    ]);
  });

  it("localizes the OpenSpec Agent node titles in Chinese", () => {
    const workflow = createMockWorkflows("zh-CN").find(
      (candidate) => candidate.id === "spec-change-lifecycle",
    );

    expect(workflow?.nodes.map((node) => node.data.title)).toEqual([
      "开始",
      "探索",
      "SFMEA检查",
      "提案",
      "实施",
      "代码缺陷扫描",
      "缺陷修复",
      "归档",
    ]);
  });
});
