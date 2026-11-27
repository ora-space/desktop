import { describe, expect, it } from "vitest";
import type { InstalledPlugin, Skill, WorkflowSummary } from "@ora/contracts";
import {
  findImportedWorkflowsFromPlugins,
  findWorkflowDependents,
  hasWorkflowSourceCandidate,
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
    expect(targets).toEqual([{ kind: "skill", id: "skill-release-notes" }]);
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
      targets: [{ kind: "skill", id: "skill-release-notes" }],
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
      targets: [{ kind: "skill", id: "skill-release-notes" }],
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
      targets: [{ kind: "skill", id: "skill-release-notes" }],
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
      targets: [{ kind: "skill", id: "skill-release-notes" }],
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
      targets: [{ kind: "skill", id: "skill-release-notes" }],
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
      targets: [{ kind: "skill", id: "skill-release-notes" }],
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
      targets: [{ kind: "skill", id: "skill-release-notes" }],
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

function workflowSummary(
  id: string,
  name: string,
  sourcePluginId: string | null,
): WorkflowSummary {
  return {
    id,
    namespace: "local",
    name,
    publishedVersion: "1.0.0",
    sourcePluginId,
    createdAt: 1n,
    updatedAt: 1n,
  };
}

describe("hasWorkflowSourceCandidate", () => {
  it("recognizes workflow plugins and unknown Pack members while skipping non-workflow plugins", () => {
    const workflowPlugin: InstalledPlugin = {
      ...mcpPlugin("official/wf-pack"),
      kind: "workflow",
    };
    expect(
      hasWorkflowSourceCandidate(
        ["official/wf-pack"],
        [workflowPlugin, skillPlugin("official/skill-pack")],
      ),
    ).toBe(true);
    expect(
      hasWorkflowSourceCandidate(
        ["official/skill-pack", "official/mcp-pack"],
        [skillPlugin("official/skill-pack"), mcpPlugin("official/mcp-pack")],
      ),
    ).toBe(false);
    expect(
      hasWorkflowSourceCandidate(["official/unknown-member"], [workflowPlugin]),
    ).toBe(true);
  });
});

describe("findImportedWorkflowsFromPlugins", () => {
  it("lists surviving workflows matching the canonical plugin id, preserving renamed entries", () => {
    const imported = findImportedWorkflowsFromPlugins(
      [
        workflowSummary(
          "wf-renamed",
          "用户已改名流程",
          "official/ora.cdase-workflow",
        ),
        workflowSummary("wf-other-ns", "同名跨源", "other/ora.cdase-workflow"),
        workflowSummary("wf-local", "本地手工", null),
        workflowSummary(
          "wf-second",
          "第二个流程",
          "official/ora.cdase-workflow",
        ),
      ],
      ["official/ora.cdase-workflow"],
    );

    expect(imported).toEqual([
      { workflowId: "wf-renamed", name: "用户已改名流程" },
      { workflowId: "wf-second", name: "第二个流程" },
    ]);
  });
});

it("compares exact IDs, legacy names, and single/multi Skill plugin references before removal", () => {
  const pluginId = "official/release";
  const pLint = {
    ...skillOwnedBy("lint", pluginId),
    id: `plugin:${pluginId}:lint`,
    namespace: pluginId,
  };
  const pReview = {
    ...skillOwnedBy("review", pluginId),
    id: `plugin:${pluginId}:review`,
    namespace: pluginId,
  };
  const localLint: Skill = {
    ...pLint,
    id: "local-lint",
    namespace: "local",
    source: { kind: "local" },
  };
  const skills = [pLint, pReview, localLint];
  const targets = uninstallDependencyTargets(
    [pluginId],
    [skillPlugin(pluginId)],
    skills,
  );
  const references = [pLint.id, pluginId, "ns:REVIEW", "lint", "unrelated"];
  const result = findWorkflowDependents({
    workflows: references.map((reference) => ({
      workflowId: reference,
      name: reference,
      draftGraph: graph({ skills: [reference] }),
    })),
    plugins: [skillPlugin(pluginId)],
    skills,
    targets,
  });
  expect(result).toEqual({
    dependents: references.slice(0, 3).map((reference) => ({
      workflowId: reference,
      name: reference,
      nodeTitles: ["发布说明"],
    })),
    unreadable: 0,
  });
});

it("ignores disabled Skill bindings and their node titles while retaining enabled and published references", () => {
  const pluginId = "official/release";
  const skill = skillOwnedBy("release-notes", pluginId);
  const disabled = JSON.parse(graph({ skills: [skill.id] }, "Disabled"));
  disabled.nodes[0].data.agentConfig.skills[0].enabled = false;
  const enabled = JSON.parse(graph({ skills: [skill.id] }, "Enabled"));
  const mixed = {
    ...disabled,
    nodes: [...disabled.nodes, { ...enabled.nodes[0], id: "enabled" }],
  };
  const result = findWorkflowDependents({
    workflows: [
      {
        workflowId: "disabled",
        name: "disabled",
        draftGraph: JSON.stringify(disabled),
      },
      { workflowId: "mixed", name: "mixed", draftGraph: JSON.stringify(mixed) },
      {
        workflowId: "published",
        name: "published",
        draftGraph: JSON.stringify(disabled),
        publishedGraph: JSON.stringify(enabled),
      },
    ],
    plugins: [],
    skills: [skill],
    targets: [{ kind: "skill", id: skill.id }],
  });
  expect(result).toEqual({
    dependents: [
      { workflowId: "mixed", name: "mixed", nodeTitles: ["Enabled"] },
      { workflowId: "published", name: "published", nodeTitles: ["Enabled"] },
    ],
    unreadable: 0,
  });
});

it("keeps Agent dependencies when unused non-Agent nodes retain malformed Skill config", () => {
  const skill = skillOwnedBy("release-notes", "official/release");
  const document = JSON.parse(graph({ skills: [skill.id] }, "Active Agent"));
  document.nodes.push({
    id: "spare-output",
    type: "workflow",
    position: { x: 100, y: 0 },
    data: { kind: "output", title: "Spare", agentConfig: { skills: {} } },
  });
  expect(
    findWorkflowDependents({
      workflows: [
        {
          workflowId: "workflow",
          name: "workflow",
          draftGraph: JSON.stringify(document),
        },
      ],
      plugins: [],
      skills: [skill],
      targets: [{ kind: "skill", id: skill.id }],
    }),
  ).toEqual({
    dependents: [
      {
        workflowId: "workflow",
        name: "workflow",
        nodeTitles: ["Active Agent"],
      },
    ],
    unreadable: 0,
  });
});

it("does not add uninstall impact when a bare Skill remains ambiguous after removal", () => {
  const pluginId = "official/review";
  const removed = {
    ...skillOwnedBy("review", pluginId),
    id: "plugin:official/review:review",
  };
  const local = {
    ...removed,
    id: "local-review",
    source: { kind: "local" as const },
  };
  const remaining = {
    ...removed,
    id: "plugin:other/review:review",
    source: { kind: "plugin" as const, pluginId: "other/review" },
  };
  expect(
    findWorkflowDependents({
      workflows: [
        {
          workflowId: "review",
          name: "Review",
          draftGraph: graph({ skills: ["review"] }),
        },
      ],
      plugins: [],
      skills: [removed, local, remaining],
      targets: [{ kind: "skill", id: removed.id }],
    }),
  ).toEqual({ dependents: [], unreadable: 0 });
});
