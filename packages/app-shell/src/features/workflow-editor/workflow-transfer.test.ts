import { describe, expect, it } from "vitest";
import type { InstalledPlugin, Skill } from "@ora/contracts";
import { createMockWorkflow } from "@ora/workflow-mock";
import {
  parseWorkflowGraph,
  serializeWorkflowGraph,
} from "@ora/workflow-runtime";
import {
  collectWorkflowDependencies,
  importPublishVersion,
  parseWorkflowImportFile,
  summarizeWorkflowTransfer,
  workflowExportFileName,
  workflowExportDocument,
} from "./workflow-transfer";

/** Builds an MCP plugin row with only the fields readiness depends on varying. */
function mcpPlugin(
  id: string,
  displayName: string,
  overrides: Partial<InstalledPlugin> = {},
): InstalledPlugin {
  return {
    id,
    namespace: id.split("/")[0],
    name: id.split("/")[1],
    displayName,
    version: "1.0.0",
    description: "",
    homepage: null,
    license: null,
    logo: null,
    installationValidity: { validity: "valid" },
    configuration: { state: "not_declared" },
    kind: "mcp",
    ...overrides,
  } as InstalledPlugin;
}

/** Builds one catalog skill. */
function skill(name: string, availability: Skill["availability"]): Skill {
  return {
    id: name,
    namespace: "local",
    name,
    description: "",
    source: { kind: "local" },
    availability,
  } as Skill;
}

/** Builds an Agent node carrying the given bindings. */
function agentNode(title: string, mcpIds: string[], skillIds: string[]) {
  return {
    data: {
      kind: "agent",
      title,
      agentConfig: {
        mcps: mcpIds.map((mcpId) => ({ mcpId, enabled: true })),
        skills: skillIds.map((skillId) => ({ skillId, enabled: false })),
      },
    },
  };
}

describe("parseWorkflowImportFile", () => {
  it("rejects files that are not JSON objects", () => {
    expect(parseWorkflowImportFile("[]")).toEqual({
      ok: false,
      failure: { reason: "invalidJson", location: null },
    });
  });

  it("locates JSON syntax errors with the surrounding lines", () => {
    const result = parseWorkflowImportFile(
      '{\n  "name": "Flow"\n  "nodes": []\n}',
    );
    expect(result.ok).toBe(false);
    if (result.ok || result.failure.reason !== "invalidJson") {
      throw new Error("expected a JSON syntax failure");
    }
    // Engines that omit the offset still produce a readable, location-free failure.
    if (result.failure.location !== null) {
      expect(result.failure.location).toEqual({
        line: 3,
        column: 3,
        excerpt: [
          { number: 2, text: '  "name": "Flow"' },
          { number: 3, text: '  "nodes": []' },
          { number: 4, text: "}" },
        ],
      });
    }
  });

  it("rejects graphs with node kinds this editor cannot render", () => {
    expect(
      parseWorkflowImportFile(
        JSON.stringify({
          name: "Flow",
          nodes: [{ data: { kind: "teleport" } }],
          edges: [],
        }),
      ),
    ).toEqual({
      ok: false,
      failure: { reason: "unknownNodeKind", kind: "teleport" },
    });
  });

  it("rejects envelopes without a name or graph arrays", () => {
    expect(
      parseWorkflowImportFile(
        JSON.stringify({ name: " ", nodes: [], edges: [] }),
      ),
    ).toEqual({ ok: false, failure: { reason: "missingName" } });
    expect(
      parseWorkflowImportFile(JSON.stringify({ name: "Flow", nodes: [] })),
    ).toEqual({ ok: false, failure: { reason: "missingGraph" } });
  });

  it("fills optional metadata so the runtime normalizer receives strings", () => {
    expect(
      parseWorkflowImportFile(
        JSON.stringify({ name: "Flow", nodes: [], edges: [] }),
      ),
    ).toEqual({
      ok: true,
      workflow: {
        name: "Flow",
        nodes: [],
        edges: [],
        id: "",
        description: "",
        updatedAt: "",
        viewport: { x: 0, y: 0, zoom: 1 },
      },
    });
  });
});

describe("workflow inactivity setting transfer", () => {
  it.each([undefined, null, "timeout", "wait"] as const)(
    "exports and imports the frozen snapshot policy %s despite later draft edits",
    (promptInactivity) => {
      const workflow = createMockWorkflow("en-US");
      const agent = workflow.nodes.find((node) => node.data.kind === "agent")!;
      if (promptInactivity !== undefined) {
        agent.data.agentConfig!.promptInactivity = promptInactivity;
      }
      const snapshot = parseWorkflowGraph(
        serializeWorkflowGraph({
          ...workflow,
          edges: workflow.edges.map(({ id, source, target }) => ({
            id,
            source,
            target,
          })),
        }),
      );
      agent.data.agentConfig!.promptInactivity = "timeout";

      const exported = workflowExportDocument(workflow, snapshot);
      const imported = parseWorkflowImportFile(JSON.stringify(exported));

      expect(imported).toEqual({ ok: true, workflow: exported });
      expect(exported.nodes).toEqual(snapshot.nodes);
      if (promptInactivity === undefined) {
        expect(
          exported.nodes.find((node) => node.id === agent.id)!.data.agentConfig,
        ).not.toHaveProperty("promptInactivity");
      }
    },
  );
});

describe("collectWorkflowDependencies", () => {
  it("merges repeated bindings and sorts actionable rows first", () => {
    const nodes = [
      { data: { kind: "start", title: "Start" } },
      agentNode("Analyze", ["ora/github"], ["review"]),
      agentNode("Fix", ["ora/github", "acme/missing"], ["broken"]),
    ];
    const plugins = [
      mcpPlugin("ora/github", "GitHub"),
      mcpPlugin("ora/sentry", "Sentry"),
    ];
    const skills = [
      skill("review", "available"),
      skill("broken", "unavailable"),
    ];

    expect(collectWorkflowDependencies(nodes, plugins, skills)).toEqual([
      {
        kind: "mcp",
        id: "acme/missing",
        label: "acme/missing",
        status: "missing",
        fix: "install",
        enabled: true,
        nodeTitles: ["Fix"],
      },
      {
        kind: "skill",
        id: "broken",
        label: "broken",
        status: "unavailable",
        fix: "reviewSkill",
        enabled: false,
        nodeTitles: ["Fix"],
      },
      {
        kind: "mcp",
        id: "ora/github",
        label: "GitHub",
        status: "installed",
        fix: null,
        enabled: true,
        nodeTitles: ["Analyze", "Fix"],
      },
      {
        kind: "skill",
        id: "review",
        label: "review",
        status: "installed",
        fix: null,
        enabled: false,
        nodeTitles: ["Analyze"],
      },
    ]);
  });

  it("routes unusable MCPs to configuration or the plugin manager", () => {
    const plugins = [
      mcpPlugin("ora/tavily", "Tavily", {
        configuration: { state: "available", completeness: "incomplete" },
      }),
      mcpPlugin("ora/broken", "Broken", {
        installationValidity: {
          validity: "invalid_declaration",
          errorCode: "manifest_invalid",
        },
      }),
    ];
    expect(
      collectWorkflowDependencies(
        [agentNode("Search", ["ora/tavily", "ora/broken"], [])],
        plugins,
        [],
      ).map(({ id, status, fix }) => ({ id, status, fix })),
    ).toEqual([
      { id: "ora/tavily", status: "unavailable", fix: "configureMcp" },
      { id: "ora/broken", status: "unavailable", fix: "manageMcp" },
    ]);
  });

  it("treats invalid MCP packages as installed but unavailable", () => {
    const plugins = [
      mcpPlugin("ora/github", "GitHub", {
        installationValidity: {
          validity: "invalid_declaration",
          errorCode: "manifest_invalid",
        },
      }),
    ];
    expect(
      collectWorkflowDependencies(
        [agentNode("Analyze", ["ora/github"], [])],
        plugins,
        [],
      ).map((dependency) => dependency.status),
    ).toEqual(["unavailable"]);
  });
});

describe("summarizeWorkflowTransfer", () => {
  it("counts executable nodes, Agent nodes, and globals", () => {
    expect(
      summarizeWorkflowTransfer({
        nodes: [
          { data: { kind: "start", title: "Start" } },
          agentNode("Analyze", [], []),
        ],
        globalVariables: [{}, {}],
      }),
    ).toEqual({ nodeCount: 2, agentCount: 1, globalVariableCount: 2 });
  });
});

describe("export and import file names", () => {
  it("embeds the published version and round-trips it on import", () => {
    const fileName = workflowExportFileName("Release: check", "v1.2.0");
    expect(fileName).toBe("Release  check.v1.2.0.reactflow.json");
    expect(importPublishVersion(fileName, "Release: check")).toBe("v1.2.0");
  });

  it("keeps the draft export name and falls back to the stem", () => {
    expect(workflowExportFileName("Flow")).toBe("Flow.reactflow.json");
    expect(workflowExportFileName("///")).toBe("workflow.reactflow.json");
    expect(importPublishVersion("Flow.reactflow.json", "Flow")).toBe("Flow");
  });

  it("lets the backend mint a version when the candidate is reserved", () => {
    expect(importPublishVersion("draft.json", "draft")).toBeNull();
  });
});
