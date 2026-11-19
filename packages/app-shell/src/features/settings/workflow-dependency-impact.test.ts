import { describe, expect, it } from "vitest";
import type { InstalledPlugin, Skill } from "@ora/contracts";
import {
  findWorkflowDependents,
  uninstallDependencyTargets,
} from "./workflow-dependency-impact";

/** Minimal installed MCP the readiness resolver can read. */
function mcpPlugin(id: string): InstalledPlugin {
  return {
    id,
    namespace: "official",
    name: id,
    displayName: id,
    version: "1.0.0",
    description: "",
    homepage: null,
    license: null,
    kind: "mcp",
    logo: null,
    installationValidity: { validity: "valid" },
    configuration: { state: "not_declared" },
    runtime: "stopped",
  };
}

/** Minimal installed Skill package; its catalog entries carry the names graphs bind. */
function skillPlugin(id: string): InstalledPlugin {
  return {
    ...mcpPlugin(id),
    kind: "skill",
    configuration: { state: "available", completeness: "complete" },
  };
}

/** One catalog Skill owned by a plugin package. */
function skillOwnedBy(name: string, pluginId: string): Skill {
  return {
    id: `skill-${name}`,
    namespace: "local",
    name,
    description: "",
    source: { kind: "plugin", pluginId },
    availability: "available",
  };
}

/** One Agent node graph envelope binding the given Skills and MCPs. */
function graph(
  bindings: { skills?: string[]; mcps?: string[] } = {},
  title = "发布说明",
): string {
  return JSON.stringify({
    viewport: { x: 0, y: 0, zoom: 1 },
    nodes: [
      {
        id: "agent-1",
        type: "workflow",
        position: { x: 0, y: 0 },
        data: {
          kind: "agent",
          title,
          agentConfig: {
            skills: (bindings.skills ?? []).map((skillId) => ({
              skillId,
              enabled: true,
            })),
            mcps: (bindings.mcps ?? []).map((mcpId) => ({
              mcpId,
              enabled: true,
            })),
          },
        },
      },
    ],
    edges: [],
    annotations: [],
    globalVariables: [],
  });
}

describe("uninstallDependencyTargets", () => {
  it("maps a Skill package to the catalog names workflows bind", () => {
    const targets = uninstallDependencyTargets(
      ["official/release"],
      [skillPlugin("official/release")],
      [
        skillOwnedBy("release-notes", "official/release"),
        skillOwnedBy("user-owned", "another-owner"),
      ],
    );
    expect(targets).toEqual([{ kind: "skill", name: "release-notes" }]);
  });

  it("keeps an MCP package bound by its own id", () => {
    const targets = uninstallDependencyTargets(
      ["official/mcp-weather"],
      [mcpPlugin("official/mcp-weather")],
      [],
    );
    expect(targets).toEqual([{ kind: "mcp", id: "official/mcp-weather" }]);
  });

  it("keeps a package the installed catalog no longer describes reachable", () => {
    const targets = uninstallDependencyTargets(["official/gone"], [], []);
    expect(targets).toEqual([{ kind: "mcp", id: "official/gone" }]);
  });
});

describe("findWorkflowDependents", () => {
  it("names each workflow and node that binds the Skill being removed", () => {
    const skills = [skillOwnedBy("release-notes", "official/release")];
    const impact = findWorkflowDependents({
      workflows: [
        {
          workflowId: "workflow-1",
          name: "发布流程",
          draftGraph: graph({ skills: ["release-notes"] }),
        },
        {
          workflowId: "workflow-2",
          name: "回归流程",
          draftGraph: graph(),
        },
      ],
      plugins: [skillPlugin("official/release")],
      skills,
      targets: [{ kind: "skill", name: "release-notes" }],
    });

    expect(impact).toEqual({
      dependents: [
        {
          workflowId: "workflow-1",
          name: "发布流程",
          nodeTitles: ["发布说明"],
        },
      ],
      unreadable: 0,
    });
  });

  it("matches an MCP by the id the graph stores", () => {
    const impact = findWorkflowDependents({
      workflows: [
        {
          workflowId: "workflow-1",
          name: "发布流程",
          draftGraph: graph({ mcps: ["official/mcp-weather"] }),
        },
      ],
      plugins: [mcpPlugin("official/mcp-weather")],
      skills: [],
      targets: [{ kind: "mcp", id: "official/mcp-weather" }],
    });

    expect(impact.dependents.map((dependent) => dependent.workflowId)).toEqual([
      "workflow-1",
    ]);
  });

  it("checks the published snapshot even after the draft drops the binding", () => {
    const skills = [skillOwnedBy("release-notes", "official/release")];
    const impact = findWorkflowDependents({
      workflows: [
        {
          workflowId: "workflow-1",
          name: "发布流程",
          draftGraph: graph(),
          publishedGraph: graph({ skills: ["release-notes"] }, "线上节点"),
        },
      ],
      plugins: [skillPlugin("official/release")],
      skills,
      targets: [{ kind: "skill", name: "release-notes" }],
    });

    expect(impact).toEqual({
      dependents: [
        {
          workflowId: "workflow-1",
          name: "发布流程",
          nodeTitles: ["线上节点"],
        },
      ],
      unreadable: 0,
    });
  });

  it("merges draft and published node titles without duplicating shared nodes", () => {
    const skills = [skillOwnedBy("release-notes", "official/release")];
    const impact = findWorkflowDependents({
      workflows: [
        {
          workflowId: "workflow-1",
          name: "发布流程",
          draftGraph: JSON.stringify({
            viewport: { x: 0, y: 0, zoom: 1 },
            nodes: [
              {
                id: "agent-1",
                type: "workflow",
                position: { x: 0, y: 0 },
                data: {
                  kind: "agent",
                  title: "共享节点",
                  agentConfig: {
                    skills: [{ skillId: "release-notes", enabled: true }],
                    mcps: [],
                  },
                },
              },
              {
                id: "agent-2",
                type: "workflow",
                position: { x: 0, y: 0 },
                data: {
                  kind: "agent",
                  title: "草稿节点",
                  agentConfig: {
                    skills: [{ skillId: "release-notes", enabled: true }],
                    mcps: [],
                  },
                },
              },
            ],
            edges: [],
            annotations: [],
            globalVariables: [],
          }),
          publishedGraph: JSON.stringify({
            viewport: { x: 0, y: 0, zoom: 1 },
            nodes: [
              {
                id: "agent-1",
                type: "workflow",
                position: { x: 0, y: 0 },
                data: {
                  kind: "agent",
                  title: "共享节点",
                  agentConfig: {
                    skills: [{ skillId: "release-notes", enabled: true }],
                    mcps: [],
                  },
                },
              },
              {
                id: "agent-3",
                type: "workflow",
                position: { x: 0, y: 0 },
                data: {
                  kind: "agent",
                  title: "已发布节点",
                  agentConfig: {
                    skills: [{ skillId: "release-notes", enabled: true }],
                    mcps: [],
                  },
                },
              },
            ],
            edges: [],
            annotations: [],
            globalVariables: [],
          }),
        },
      ],
      plugins: [skillPlugin("official/release")],
      skills,
      targets: [{ kind: "skill", name: "release-notes" }],
    });

    expect(impact).toEqual({
      dependents: [
        {
          workflowId: "workflow-1",
          name: "发布流程",
          nodeTitles: ["共享节点", "草稿节点", "已发布节点"],
        },
      ],
      unreadable: 0,
    });
  });

  it("counts a workflow when its published graph is unreadable without losing draft hits", () => {
    const skills = [skillOwnedBy("release-notes", "official/release")];
    const impact = findWorkflowDependents({
      workflows: [
        {
          workflowId: "workflow-1",
          name: "发布流程",
          draftGraph: graph(),
          publishedGraph: "[]",
        },
        {
          workflowId: "workflow-2",
          name: "双坏流程",
          draftGraph: "not json",
          publishedGraph: "null",
        },
        {
          workflowId: "workflow-3",
          name: "草稿命中流程",
          draftGraph: graph({ skills: ["release-notes"] }),
          publishedGraph: "not json",
        },
      ],
      plugins: [skillPlugin("official/release")],
      skills,
      targets: [{ kind: "skill", name: "release-notes" }],
    });

    expect(impact).toEqual({
      dependents: [
        {
          workflowId: "workflow-3",
          name: "草稿命中流程",
          nodeTitles: ["发布说明"],
        },
      ],
      unreadable: 3,
    });
  });

  it("counts valid JSON graphs that are not objects as unreadable", () => {
    const impact = findWorkflowDependents({
      workflows: [
        { workflowId: "workflow-1", name: "数组图", draftGraph: "[]" },
        { workflowId: "workflow-2", name: "空值图", draftGraph: "null" },
        { workflowId: "workflow-3", name: "数字图", draftGraph: "42" },
        { workflowId: "workflow-4", name: "文本图", draftGraph: '"text"' },
        { workflowId: "workflow-5", name: "布尔图", draftGraph: "true" },
        { workflowId: "workflow-6", name: "缺失图", draftGraph: undefined },
        { workflowId: "workflow-7", name: "坏JSON", draftGraph: "not json" },
      ],
      plugins: [skillPlugin("official/release")],
      skills: [],
      targets: [{ kind: "skill", name: "release-notes" }],
    });

    expect(impact).toEqual({ dependents: [], unreadable: 7 });
  });

  it("reads an explicit null agent config as a workflow with no dependencies", () => {
    const impact = findWorkflowDependents({
      workflows: [
        {
          workflowId: "workflow-null",
          name: "空配置流程",
          draftGraph: JSON.stringify({
            viewport: { x: 0, y: 0, zoom: 1 },
            nodes: [
              {
                id: "agent-null",
                type: "workflow",
                position: { x: 0, y: 0 },
                data: {
                  kind: "agent",
                  title: "空配置节点",
                  agentConfig: null,
                },
              },
            ],
            edges: [],
            annotations: [],
            globalVariables: [],
          }),
        },
        {
          workflowId: "workflow-good",
          name: "发布流程",
          draftGraph: graph({ skills: ["release-notes"] }),
        },
      ],
      plugins: [skillPlugin("official/release")],
      skills: [skillOwnedBy("release-notes", "official/release")],
      targets: [{ kind: "skill", name: "release-notes" }],
    });

    // A null config declares no bindings; the install report reads it the same way.
    expect(impact).toEqual({
      dependents: [
        {
          workflowId: "workflow-good",
          name: "发布流程",
          nodeTitles: ["发布说明"],
        },
      ],
      unreadable: 0,
    });
  });

  it("counts a malformed node whose bindings cannot be read as unreadable without aborting", () => {
    const skills = [skillOwnedBy("release-notes", "official/release")];
    const malformedGraph = JSON.stringify({
      viewport: { x: 0, y: 0, zoom: 1 },
      nodes: [
        {
          id: "agent-bad",
          type: "workflow",
          position: { x: 0, y: 0 },
          data: {
            kind: "agent",
            title: "坏配置节点",
            // Not a binding list: the resolver throws, so the workflow stays unknown.
            agentConfig: { mcps: 42 },
          },
        },
      ],
      edges: [],
      annotations: [],
      globalVariables: [],
    });
    const impact = findWorkflowDependents({
      workflows: [
        {
          workflowId: "workflow-bad",
          name: "异常节点流程",
          draftGraph: malformedGraph,
        },
        {
          workflowId: "workflow-good",
          name: "发布流程",
          draftGraph: graph({ skills: ["release-notes"] }),
        },
      ],
      plugins: [skillPlugin("official/release")],
      skills,
      targets: [{ kind: "skill", name: "release-notes" }],
    });

    expect(impact).toEqual({
      dependents: [
        {
          workflowId: "workflow-good",
          name: "发布流程",
          nodeTitles: ["发布说明"],
        },
      ],
      unreadable: 1,
    });
  });
});
