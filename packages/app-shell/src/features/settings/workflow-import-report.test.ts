import { describe, expect, it } from "vitest";
import type {
  ImportedWorkflowOutcome,
  InstalledPlugin,
  Skill,
} from "@ora/contracts";
import {
  buildWorkflowImportReport,
  importedArchiveName,
} from "./workflow-import-report";

/** One Agent node bound to the given Skills and MCPs, shaped like an exported graph envelope. */
function agentGraph(bindings: { skills?: string[]; mcps?: string[] } = {}) {
  return JSON.stringify({
    viewport: { x: 0, y: 0, zoom: 1 },
    nodes: [
      {
        id: "agent-1",
        type: "workflow",
        position: { x: 0, y: 0 },
        data: {
          kind: "agent",
          title: "编写发布说明",
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

/** One successfully imported document, as the backend reports it. */
function imported(
  workflowId: string,
  name: string,
  sourceFile = "assets/workflows/1.0.0.json",
): ImportedWorkflowOutcome {
  return { state: "imported", sourceFile, workflowId, name, version: "1.0.0" };
}

/** A usable installed MCP package. */
function installedMcp(id: string): InstalledPlugin {
  return {
    id,
    namespace: "official",
    name: id.slice(id.indexOf("/") + 1),
    displayName: "Weather MCP",
    version: "1.0.0",
    description: "Weather tools",
    homepage: null,
    license: null,
    logo: null,
    installationValidity: { validity: "valid" },
    configuration: { state: "available", completeness: "complete" },
    runtime: "stopped",
    kind: "mcp",
  };
}

/** A catalog Skill whose on-disk package is still loadable. */
function availableSkill(name: string): Skill {
  return {
    id: `skill-${name}`,
    namespace: "local",
    name,
    description: "Writes release notes",
    source: { kind: "local" },
    availability: "available",
  };
}

describe("buildWorkflowImportReport", () => {
  it("resolves each created workflow's bound plugins against the local catalogs", () => {
    const report = buildWorkflowImportReport({
      fileName: "workflows.orax",
      outcomes: [imported("wf-1", "发布流程")],
      graphs: new Map([
        [
          "wf-1",
          agentGraph({
            skills: ["release-notes"],
            mcps: ["official/weather"],
          }),
        ],
      ]),
      plugins: [installedMcp("official/weather")],
      skills: [],
    });

    expect(report.workflows).toHaveLength(1);
    // Missing entries sort first, so the actionable row leads.
    expect(
      report.workflows[0]?.dependencies?.map((dependency) => [
        dependency.kind,
        dependency.id,
        dependency.status,
        dependency.nodeTitles,
      ]),
    ).toEqual([
      ["skill", "release-notes", "missing", ["编写发布说明"]],
      ["mcp", "official/weather", "installed", ["编写发布说明"]],
    ]);
    expect(report.counts).toEqual({
      installed: 1,
      unavailable: 0,
      missing: 1,
    });
    expect(report.refused).toEqual([]);
  });

  it("counts a plugin two workflows share once, keeping both workflow sections", () => {
    const report = buildWorkflowImportReport({
      fileName: "workflows.orax",
      outcomes: [imported("wf-1", "发布流程"), imported("wf-2", "回归流程")],
      graphs: new Map([
        ["wf-1", agentGraph({ skills: ["release-notes"] })],
        ["wf-2", agentGraph({ skills: ["release-notes"] })],
      ]),
      plugins: [],
      skills: [availableSkill("release-notes")],
    });

    expect(report.workflows).toHaveLength(2);
    expect(report.counts).toEqual({
      installed: 1,
      unavailable: 0,
      missing: 0,
    });
  });

  it("keeps a workflow whose graph could not be read instead of claiming it needs nothing", () => {
    const report = buildWorkflowImportReport({
      fileName: "workflows.orax",
      outcomes: [imported("wf-1", "发布流程")],
      graphs: new Map(),
      plugins: [],
      skills: [],
    });

    expect(report.workflows[0]?.dependencies).toBeNull();
    expect(report.counts).toEqual({
      installed: 0,
      unavailable: 0,
      missing: 0,
    });
  });

  it("reports a refused document with the path and reason the backend gave", () => {
    const report = buildWorkflowImportReport({
      fileName: "workflows.orax",
      outcomes: [
        {
          state: "failed",
          sourceFile: "assets/workflows/2.0.0.json",
          reason: "document is not valid JSON",
        },
      ],
      graphs: new Map(),
      plugins: [],
      skills: [],
    });

    expect(report.refused).toEqual([
      {
        sourceFile: "assets/workflows/2.0.0.json",
        reason: "document is not valid JSON",
      },
    ]);
    expect(report.workflows).toEqual([]);
  });
});

describe("importedArchiveName", () => {
  it("takes the last segment on either separator the host may report", () => {
    expect(importedArchiveName("C:\\downloads\\workflows.orax")).toBe(
      "workflows.orax",
    );
    expect(importedArchiveName("/home/me/workflows.orax")).toBe(
      "workflows.orax",
    );
    expect(importedArchiveName("workflows.orax")).toBe("workflows.orax");
  });
});
