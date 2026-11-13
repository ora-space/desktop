import type { InstalledPlugin, Skill } from "@ora/contracts";
import {
  WORKFLOW_NODE_KINDS,
  workflowAgentCompletionIssues,
  type DemoWorkflow,
} from "@ora/workflow-mock";

/** Largest workflow file accepted for preview; larger files are rejected before parsing. */
export const MAX_WORKFLOW_IMPORT_BYTES = 5 * 1024 * 1024;

/** One source line shown around a JSON syntax error. */
export interface WorkflowJsonExcerptLine {
  number: number;
  text: string;
}

/** Where JSON parsing stopped, when the runtime reports a character position. */
export interface WorkflowJsonErrorLocation {
  line: number;
  column: number;
  excerpt: WorkflowJsonExcerptLine[];
}

/** Why a selected file cannot become an import preview. */
export type WorkflowImportFailure =
  | { reason: "invalidJson"; location: WorkflowJsonErrorLocation | null }
  | { reason: "missingName" }
  | { reason: "missingGraph" }
  | { reason: "unknownNodeKind"; kind: string }
  | { reason: "invalidAgentConfig"; nodeTitle: string; detail: string }
  | { reason: "fileTooLarge" };

/** Outcome of reading one exported workflow file before anything is persisted. */
export type WorkflowImportParseResult =
  | { ok: true; workflow: DemoWorkflow }
  | { ok: false; failure: WorkflowImportFailure };

/** Plugin families a workflow graph references by identity. */
export type WorkflowDependencyKind = "mcp" | "skill";

/**
 * Local readiness of one referenced plugin.
 * `unavailable` means installed but not usable (invalid package or incomplete configuration).
 */
export type WorkflowDependencyStatus = "installed" | "unavailable" | "missing";

/**
 * The action that resolves a non-ready dependency: install a missing MCP or Skill from the
 * marketplace, open an MCP's configuration, inspect an MCP whose package declaration is
 * invalid in the plugin manager, or review an unusable Skill on the Skills page.
 */
export type WorkflowDependencyFix =
  "install" | "configureMcp" | "manageMcp" | "reviewSkill";

/** One plugin reference aggregated across every Agent node that binds it. */
export interface WorkflowDependency {
  kind: WorkflowDependencyKind;
  /** Graph identity: the full plugin ID for MCP, the skill name for Skill. */
  id: string;
  /** Catalog display name when installed, otherwise the stored identity. */
  label: string;
  status: WorkflowDependencyStatus;
  /** How to resolve the dependency; `null` once it is installed and usable. */
  fix: WorkflowDependencyFix | null;
  /** True when at least one binding of this plugin is enabled in the graph. */
  enabled: boolean;
  /** Titles of Agent nodes that reference this plugin, in graph order. */
  nodeTitles: string[];
}

/** Counts shown in the import preview so users can recognize the file's content. */
export interface WorkflowTransferSummary {
  nodeCount: number;
  agentCount: number;
  globalVariableCount: number;
}

/** Minimal node shape shared by editor workflows and parsed snapshot envelopes. */
interface WorkflowTransferNode {
  data: {
    kind: string;
    title: string;
    agentConfig?: {
      skills?: readonly { skillId: string; enabled: boolean }[];
      mcps?: readonly { mcpId: string; enabled: boolean }[];
    };
  };
}

/** Narrows unknown JSON to a plain object without accepting arrays. */
function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/**
 * Returns the import-blocking issue of an Agent configuration's completion contract.
 *
 * Only the fields this editor writes conditionally are checked: legacy files without a version
 * still import, while an unsupported version, a version-3 completion, or an unknown declaration
 * field is refused here instead of being silently dropped on the way to the backend.
 */
function agentConfigImportIssue(config: unknown): string | null {
  if (!isRecord(config)) {
    return null;
  }
  const version = config.schemaVersion;
  if (version !== undefined && version !== 3 && version !== 4) {
    return `unsupported agentConfig.schemaVersion ${String(version)}; supported versions are 3 and 4`;
  }
  const completionIssues = workflowAgentCompletionIssues(config.completion);
  if (completionIssues.length > 0) {
    return completionIssues[0]!;
  }
  if (config.completion != null && version !== 4) {
    return "agentConfig.completion requires agentConfig.schemaVersion 4";
  }
  return null;
}

/**
 * Maps a `JSON.parse` failure to a 1-based line/column and nearby lines. V8 reports a
 * character offset as "at position N"; other engines may not, so the location is optional.
 */
function jsonErrorLocation(
  text: string,
  error: unknown,
): WorkflowJsonErrorLocation | null {
  const message = error instanceof Error ? error.message : "";
  const match = /position (\d+)/.exec(message);
  if (match === null) {
    return null;
  }
  const offset = Math.min(Number(match[1]), text.length);
  const before = text.slice(0, offset).split("\n");
  const line = before.length;
  const column = before[before.length - 1].length + 1;
  const lines = text.split("\n");
  const first = Math.max(1, line - 1);
  const last = Math.min(lines.length, line + 1);
  const excerpt: WorkflowJsonExcerptLine[] = [];
  for (let number = first; number <= last; number += 1) {
    excerpt.push({ number, text: lines[number - 1].replace(/\r$/, "") });
  }
  return { line, column, excerpt };
}

/**
 * Validates the envelope written by workflow export. Deep node validation stays with the
 * runtime normalizer and run engine; this only rejects files that cannot be previewed.
 */
export function parseWorkflowImportFile(
  text: string,
): WorkflowImportParseResult {
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch (error) {
    return {
      ok: false,
      failure: {
        reason: "invalidJson",
        location: jsonErrorLocation(text, error),
      },
    };
  }
  if (!isRecord(value)) {
    return { ok: false, failure: { reason: "invalidJson", location: null } };
  }
  if (typeof value.name !== "string" || value.name.trim() === "") {
    return { ok: false, failure: { reason: "missingName" } };
  }
  if (!Array.isArray(value.nodes) || !Array.isArray(value.edges)) {
    return { ok: false, failure: { reason: "missingGraph" } };
  }
  const knownKinds: readonly string[] = WORKFLOW_NODE_KINDS;
  for (const node of value.nodes) {
    const data = isRecord(node) && isRecord(node.data) ? node.data : undefined;
    const kind = data?.kind;
    if (typeof kind !== "string" || !knownKinds.includes(kind)) {
      return {
        ok: false,
        failure: { reason: "unknownNodeKind", kind: String(kind) },
      };
    }
    if (kind === "agent") {
      const detail = agentConfigImportIssue(data?.agentConfig);
      if (detail !== null) {
        return {
          ok: false,
          failure: {
            reason: "invalidAgentConfig",
            nodeTitle:
              typeof data?.title === "string" && data.title.trim() !== ""
                ? data.title
                : String(node && isRecord(node) ? node.id : ""),
            detail,
          },
        };
      }
    }
  }
  return {
    ok: true,
    workflow: {
      ...(value as unknown as DemoWorkflow),
      id: typeof value.id === "string" ? value.id : "",
      description:
        typeof value.description === "string" ? value.description : "",
      updatedAt: typeof value.updatedAt === "string" ? value.updatedAt : "",
      viewport: isRecord(value.viewport)
        ? (value.viewport as unknown as DemoWorkflow["viewport"])
        : { x: 0, y: 0, zoom: 1 },
    },
  };
}

/** Summarizes executable content; editor annotations are not counted as nodes. */
export function summarizeWorkflowTransfer(workflow: {
  nodes: readonly WorkflowTransferNode[];
  globalVariables?: readonly unknown[];
}): WorkflowTransferSummary {
  return {
    nodeCount: workflow.nodes.length,
    agentCount: workflow.nodes.filter((node) => node.data.kind === "agent")
      .length,
    globalVariableCount: workflow.globalVariables?.length ?? 0,
  };
}

/** Resolves MCP readiness and its fix with the same rules the Agent inspector uses. */
function mcpReadiness(
  plugin: InstalledPlugin | undefined,
): Pick<WorkflowDependency, "status" | "fix"> {
  if (plugin === undefined) {
    return { status: "missing", fix: "install" };
  }
  if (plugin.installationValidity.validity !== "valid") {
    // A broken package declaration cannot be configured; the manager shows its error.
    return { status: "unavailable", fix: "manageMcp" };
  }
  if (
    plugin.configuration.state === "unavailable" ||
    (plugin.configuration.state === "available" &&
      plugin.configuration.completeness !== "complete")
  ) {
    return { status: "unavailable", fix: "configureMcp" };
  }
  return { status: "installed", fix: null };
}

/**
 * Collects every MCP and Skill binding in the graph, including disabled ones, because
 * import/export preserves disabled bindings and users should see what the file carries.
 * Missing entries sort first so the actionable rows are visible without scrolling.
 */
export function collectWorkflowDependencies(
  nodes: readonly WorkflowTransferNode[],
  plugins: readonly InstalledPlugin[],
  skills: readonly Skill[],
): WorkflowDependency[] {
  const pluginsById = new Map(
    plugins
      .filter((plugin) => plugin.kind === "mcp")
      .map((plugin) => [plugin.id, plugin]),
  );
  const skillsByName = new Map(skills.map((skill) => [skill.name, skill]));
  const dependencies = new Map<string, WorkflowDependency>();

  /** Adds one reference, merging node titles for repeated bindings. */
  function add(
    kind: WorkflowDependencyKind,
    id: string,
    nodeTitle: string,
    enabled: boolean,
    resolve: () => Pick<WorkflowDependency, "label" | "status" | "fix">,
  ): void {
    const key = `${kind}:${id}`;
    const existing = dependencies.get(key);
    if (existing !== undefined) {
      existing.enabled ||= enabled;
      if (!existing.nodeTitles.includes(nodeTitle)) {
        existing.nodeTitles.push(nodeTitle);
      }
      return;
    }
    dependencies.set(key, {
      kind,
      id,
      enabled,
      nodeTitles: [nodeTitle],
      ...resolve(),
    });
  }

  for (const node of nodes) {
    const config = node.data.agentConfig;
    if (node.data.kind !== "agent" || config === undefined) {
      continue;
    }
    for (const binding of config.mcps ?? []) {
      add("mcp", binding.mcpId, node.data.title, binding.enabled, () => {
        const plugin = pluginsById.get(binding.mcpId);
        return {
          label: plugin?.displayName ?? binding.mcpId,
          ...mcpReadiness(plugin),
        };
      });
    }
    for (const binding of config.skills ?? []) {
      add("skill", binding.skillId, node.data.title, binding.enabled, () => {
        const skill = skillsByName.get(binding.skillId);
        if (skill === undefined) {
          return { label: binding.skillId, status: "missing", fix: "install" };
        }
        return skill.availability === "available"
          ? { label: skill.name, status: "installed", fix: null }
          : { label: skill.name, status: "unavailable", fix: "reviewSkill" };
      });
    }
  }

  const rank: Record<WorkflowDependencyStatus, number> = {
    missing: 0,
    unavailable: 1,
    installed: 2,
  };
  return [...dependencies.values()].sort(
    (left, right) => rank[left.status] - rank[right.status],
  );
}

/** Replaces characters that are unsafe in file names on any desktop platform. */
function safeFileStem(value: string): string {
  // `\p{Cc}` is the Unicode Control category; property escapes keep control
  // characters out of the regex literal so no-control-regex stays satisfied.
  return value.replace(/[<>:"/\\|?*\p{Cc}]/gu, " ").trim();
}

/**
 * Produces a portable export filename. A published version is embedded before the
 * extension so importing the file proposes the same version again.
 */
export function workflowExportFileName(
  name: string,
  version: string | null = null,
): string {
  const stem = safeFileStem(name);
  const base = stem === "" ? "workflow" : stem;
  const versionStem = version === null ? "" : safeFileStem(version);
  return versionStem === ""
    ? `${base}.reactflow.json`
    : `${base}.${versionStem}.reactflow.json`;
}

/** Applies the backend's user-provided version constraints before proposing a value. */
export function isValidPublishVersion(candidate: string): boolean {
  return !(
    candidate === "" ||
    candidate === "draft" ||
    candidate === "." ||
    candidate === ".." ||
    candidate.length > 128 ||
    [...candidate].some(
      (character) =>
        character === "/" || character === "\\" || character.charCodeAt(0) < 32,
    )
  );
}

/**
 * Picks a publish version for an imported file: prefer the version embedded by export,
 * then the filename stem, then the workflow title, else let the backend mint one.
 */
export function importPublishVersion(
  fileName: string,
  workflowName: string,
): string | null {
  const stem = fileName
    .replace(/\.reactflow\.json$/i, "")
    .replace(/\.json$/i, "")
    .trim();
  const prefix = `${safeFileStem(workflowName)}.`;
  const embedded =
    stem.startsWith(prefix) && stem.length > prefix.length
      ? stem.slice(prefix.length).trim()
      : "";
  const candidate = (
    embedded !== "" ? embedded : stem !== "" ? stem : workflowName
  ).trim();
  return isValidPublishVersion(candidate) ? candidate : null;
}

/** Graph envelope fields a published snapshot contributes to an export document. */
interface WorkflowExportEnvelope {
  nodes: readonly unknown[];
  edges: readonly unknown[];
  viewport: DemoWorkflow["viewport"];
  annotations: readonly unknown[];
  globalVariables: DemoWorkflow["globalVariables"];
  description?: string;
}

/**
 * Builds the exact document written by export: the workflow identity plus either its draft
 * graph or a published snapshot's graph. Shared by the file preview and the save path.
 */
export function workflowExportDocument(
  workflow: DemoWorkflow,
  snapshot: WorkflowExportEnvelope | null,
): DemoWorkflow {
  if (snapshot === null) {
    return workflow;
  }
  return {
    id: workflow.id,
    name: workflow.name,
    description: snapshot.description ?? workflow.description,
    updatedAt: workflow.updatedAt,
    viewport: snapshot.viewport,
    nodes: snapshot.nodes as DemoWorkflow["nodes"],
    edges: snapshot.edges as DemoWorkflow["edges"],
    annotations: snapshot.annotations as DemoWorkflow["annotations"],
    globalVariables: snapshot.globalVariables,
  };
}

/** Formats a byte count the way file sizes are shown next to import file names. */
export function formatWorkflowFileSize(bytes: number): string {
  if (bytes < 1024) {
    return `${bytes} B`;
  }
  if (bytes < 1024 * 1024) {
    return `${(bytes / 1024).toFixed(1)} KB`;
  }
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}
