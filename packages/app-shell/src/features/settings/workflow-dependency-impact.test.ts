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
function graph(bindings: { skills?: string[]; mcps?: string[] } = {}): string {
  return JSON.stringify({
    viewport: { x: 0, y: 0, zoom: 1 },
    nodes: [
      {
        id: "agent-1",
        type: "workflow",
        position: { x: 0, y: 0 },
        data: {
          kind: "agent",
          title: "发布说明",
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
          graph: graph({ skills: ["release-notes"] }),
        },
        {
          workflowId: "workflow-2",
          name: "回归流程",
          graph: graph(),
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
          graph: graph({ mcps: ["official/mcp-weather"] }),
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

  it("counts a workflow whose graph cannot be read instead of clearing it", () => {
    const impact = findWorkflowDependents({
      workflows: [
        { workflowId: "workflow-1", name: "发布流程", graph: "not json" },
        { workflowId: "workflow-2", name: "回归流程", graph: undefined },
      ],
      plugins: [skillPlugin("official/release")],
      skills: [],
      targets: [{ kind: "skill", name: "release-notes" }],
    });

    expect(impact).toEqual({ dependents: [], unreadable: 2 });
  });
});
