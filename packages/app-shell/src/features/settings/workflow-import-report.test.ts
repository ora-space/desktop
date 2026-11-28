import { describe, expect, it } from "vitest";
import type {
  ImportedWorkflowOutcome,
  InstalledPlugin,
  Skill,
} from "@ora/contracts";
import {
  buildWorkflowImportReport,
  importedArchiveName,
  marketplaceWorkflowReportInput,
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
    origin: { kind: "official" },
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
      ambiguous: 0,
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
      ambiguous: 0,
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
      ambiguous: 0,
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

describe("marketplaceWorkflowReportInput", () => {
  it("preserves each Pack member id on unresolved and deleted entries", () => {
    const report = marketplaceWorkflowReportInput(
      "official/ora-space.pack",
      [
        {
          state: "skipped_conflict",
          pluginId: "official/ora-space.wf-a",
          sourceFile: "assets/workflows/1.0.0.json",
          name: "发布流程",
          knownConflict: false,
        },
        {
          state: "skipped_conflict",
          pluginId: "official/ora-space.wf-b",
          sourceFile: "assets/workflows/1.0.0.json",
          name: "回归流程",
          knownConflict: false,
        },
      ],
      "install",
    );

    expect(report).toEqual({
      fileName: "official/ora-space.pack",
      outcomes: [],
      unresolved: [
        {
          pluginId: "official/ora-space.wf-a",
          sourceFile: "assets/workflows/1.0.0.json",
          name: "发布流程",
          knownConflict: false,
        },
        {
          pluginId: "official/ora-space.wf-b",
          sourceFile: "assets/workflows/1.0.0.json",
          name: "回归流程",
          knownConflict: false,
        },
      ],
      deletedCopies: [],
    });
  });

  it("drops deleted copies from upgrade reports while keeping them on card retries", () => {
    const workflows = [
      {
        state: "skipped_conflict" as const,
        pluginId: "official/weather",
        sourceFile: "assets/workflows/1.0.0.json",
        name: "发布流程",
        knownConflict: false,
      },
      {
        state: "skipped_deleted_copy" as const,
        pluginId: "official/weather",
        sourceFile: "assets/workflows/deleted.json",
      },
    ];

    expect(
      marketplaceWorkflowReportInput("official/weather", workflows, "update"),
    ).toEqual({
      fileName: "official/weather",
      outcomes: [],
      unresolved: [
        {
          pluginId: "official/weather",
          sourceFile: "assets/workflows/1.0.0.json",
          name: "发布流程",
          knownConflict: false,
        },
      ],
      deletedCopies: [],
    });

    expect(
      marketplaceWorkflowReportInput("official/weather", workflows, "retry"),
    ).toEqual({
      fileName: "official/weather",
      outcomes: [],
      unresolved: [
        {
          pluginId: "official/weather",
          sourceFile: "assets/workflows/1.0.0.json",
          name: "发布流程",
          knownConflict: false,
        },
      ],
      deletedCopies: [
        {
          pluginId: "official/weather",
          sourceFile: "assets/workflows/deleted.json",
        },
      ],
    });
  });

  it("opens an upgrade report on a first conflict alongside known ones, and stays closed when only neutral skips exist", () => {
    expect(
      marketplaceWorkflowReportInput(
        "official/weather",
        [
          {
            state: "skipped_conflict",
            pluginId: "official/weather",
            sourceFile: "assets/workflows/1.0.0.json",
            name: "发布流程",
            knownConflict: true,
          },
          {
            state: "skipped_deleted_copy",
            pluginId: "official/weather",
            sourceFile: "assets/workflows/deleted.json",
          },
        ],
        "update",
      ),
    ).toBeNull();

    expect(
      marketplaceWorkflowReportInput(
        "official/weather",
        [
          {
            state: "skipped_conflict",
            pluginId: "official/weather",
            sourceFile: "assets/workflows/1.0.0.json",
            name: "发布流程",
            knownConflict: false,
          },
          {
            state: "skipped_conflict",
            pluginId: "official/weather",
            sourceFile: "assets/workflows/2.0.0.json",
            name: "回归流程",
            knownConflict: true,
          },
        ],
        "update",
      ),
    ).toEqual({
      fileName: "official/weather",
      outcomes: [],
      unresolved: [
        {
          pluginId: "official/weather",
          sourceFile: "assets/workflows/1.0.0.json",
          name: "发布流程",
          knownConflict: false,
        },
        {
          pluginId: "official/weather",
          sourceFile: "assets/workflows/2.0.0.json",
          name: "回归流程",
          knownConflict: true,
        },
      ],
      deletedCopies: [],
    });
  });
});

it("keeps disabled missing references in details without counting them as blockers", () => {
  const disabled = JSON.parse(
    agentGraph({ skills: ["absent"], mcps: ["official/absent"] }),
  );
  disabled.nodes[0].data.agentConfig.skills[0].enabled = false;
  disabled.nodes[0].data.agentConfig.mcps[0].enabled = false;
  const report = buildWorkflowImportReport({
    fileName: "disabled.orax",
    outcomes: [imported("wf", "disabled")],
    graphs: new Map([["wf", JSON.stringify(disabled)]]),
    plugins: [],
    skills: [],
  });
  expect(report.counts).toEqual({
    installed: 0,
    missing: 0,
    unavailable: 0,
    ambiguous: 0,
  });
  expect(
    report.workflows[0]?.dependencies?.map(({ status, enabled }) => ({
      status,
      enabled,
    })),
  ).toEqual([
    { status: "missing", enabled: false },
    { status: "missing", enabled: false },
  ]);
});

it("counts unavailable and ambiguous enabled Skill references independently", () => {
  const local = availableSkill("review");
  const plugin: Skill = {
    ...local,
    id: "plugin:official/review:review",
    namespace: "official/review",
    source: {
      kind: "plugin",
      pluginId: "official/review",
      origin: { kind: "official" },
    },
    availability: "unavailable",
  };
  const report = buildWorkflowImportReport({
    fileName: "skills.orax",
    outcomes: [imported("wf", "skills")],
    graphs: new Map([["wf", agentGraph({ skills: ["review", plugin.id] })]]),
    plugins: [],
    skills: [local, plugin],
  });
  expect(report.counts).toEqual({
    installed: 0,
    missing: 0,
    unavailable: 1,
    ambiguous: 1,
  });
});
