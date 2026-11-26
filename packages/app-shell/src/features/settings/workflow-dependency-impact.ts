import { resolveWorkflowSkill } from "../workflow-editor/workflow-skill-reference";
import type { InstalledPlugin, Skill, WorkflowSummary } from "@ora/contracts";
import { parseWorkflowGraph } from "@ora/workflow-runtime";
import { collectWorkflowDependencies } from "../workflow-editor/workflow-transfer";

/**
 * The identity a workflow graph binds when it references a plugin that is about to leave.
 *
 * MCP targets are plugin IDs; Skill targets are the exact catalog IDs removed from resolution.
 */
export type WorkflowDependencyTarget =
  { kind: "mcp"; id: string } | { kind: "skill"; id: string };

/** One workflow whose graph references a plugin the user is about to uninstall. */
export interface WorkflowDependent {
  workflowId: string;
  name: string;
  /** Titles of the Agent nodes that bind the plugin, in graph order. */
  nodeTitles: string[];
}

/** What an uninstall would break, plus the workflows that could not be checked. */
export interface WorkflowDependencyImpact {
  dependents: WorkflowDependent[];
  /** Workflows whose graph could not be read, so their references stay unknown. */
  unreadable: number;
}

/** One surviving workflow copy that a workflow plugin originally imported. */
export interface WorkflowImportedByPlugin {
  workflowId: string;
  name: string;
}

/**
 * Reports whether any pending removal could have imported workflows into the library.
 *
 * Unknown ids (for example a Pack member absent from the installed catalog) stay eligible so we
 * do not skip checking the library when a package record is unavailable.
 */
export function hasWorkflowSourceCandidate(
  pluginIds: readonly string[],
  plugins: readonly InstalledPlugin[],
): boolean {
  return pluginIds.some((pluginId) => {
    const plugin = plugins.find((candidate) => candidate.id === pluginId);
    return plugin === undefined || plugin.kind === "workflow";
  });
}

/**
 * Lists surviving workflows imported by any plugin in `pluginIds`, preserving library order.
 */
export function findImportedWorkflowsFromPlugins(
  workflows: readonly WorkflowSummary[],
  pluginIds: readonly string[],
): WorkflowImportedByPlugin[] {
  const targets = new Set(pluginIds);
  const imported: WorkflowImportedByPlugin[] = [];
  for (const workflow of workflows) {
    if (
      workflow.sourcePluginId !== null &&
      targets.has(workflow.sourcePluginId)
    ) {
      imported.push({ workflowId: workflow.id, name: workflow.name });
    }
  }
  return imported;
}

/**
 * Resolves the plugins an uninstall would remove to the identities workflows reference.
 *
 * Skill targets remove catalog IDs before resolving the graph again; MCP references keep their
 * plugin ID identity. Missing package records still retain their known MCP and Skill targets.
 */
export function uninstallDependencyTargets(
  pluginIds: readonly string[],
  plugins: readonly InstalledPlugin[],
  skills: readonly Skill[],
): WorkflowDependencyTarget[] {
  const targets: WorkflowDependencyTarget[] = [];
  const seen = new Set<string>();
  /** Keeps the same reference reachable by either an MCP id or a Skill catalog ID only once. */
  function add(target: WorkflowDependencyTarget): void {
    const key = `${target.kind}:${target.id}`;
    if (seen.has(key)) return;
    seen.add(key);
    targets.push(target);
  }

  for (const pluginId of pluginIds) {
    const plugin = plugins.find((candidate) => candidate.id === pluginId);
    if (plugin === undefined) {
      add({ kind: "mcp", id: pluginId });
    } else if (plugin.kind === "mcp") {
      add({ kind: "mcp", id: pluginId });
    }
    if (plugin === undefined || plugin.kind === "skill") {
      for (const skill of skills) {
        if (
          skill.source.kind === "plugin" &&
          skill.source.pluginId === pluginId
        ) {
          add({ kind: "skill", id: skill.id });
        }
      }
    }
  }
  return targets;
}

/**
 * Reports whether a snapshot graph is a JSON object the workflow codec can read at all.
 *
 * The codec deliberately turns an unparsable snapshot into an empty envelope so the canvas still
 * opens; that is the right answer for editing and the wrong answer for an uninstall check, where
 * "no nodes" would be read as "no references".
 */
function isReadableGraph(graph: string): boolean {
  try {
    const value: unknown = JSON.parse(graph);
    return typeof value === "object" && value !== null && !Array.isArray(value);
  } catch {
    return false;
  }
}

/**
 * Finds every workflow whose draft or published snapshot binds a plugin the user is about to
 * uninstall.
 *
 * Runs execute the published snapshot while editing changes the draft, so removing a binding in an
 * unpublished draft must not hide an active dependency in the published graph. Both snapshots share
 * the import report's dependency resolver, and any unreadable or malformed graph is counted once
 * per workflow rather than failing the whole scan or reading as "no references".
 */
export function findWorkflowDependents(input: {
  /** Library summaries in display order, each with the draft and published graphs loaded for it. */
  workflows: readonly {
    workflowId: string;
    name: string;
    /** `undefined` when the workflow record itself could not be fetched. */
    draftGraph: string | undefined;
    /** `null` or `undefined` when the workflow has no published snapshot. */
    publishedGraph?: string | null;
  }[];
  plugins: readonly InstalledPlugin[];
  skills: readonly Skill[];
  retiredSkillIds?: readonly string[];
  targets: readonly WorkflowDependencyTarget[];
}): WorkflowDependencyImpact {
  const dependents: WorkflowDependent[] = [];
  let unreadable = 0;

  for (const workflow of input.workflows) {
    if (workflow.draftGraph === undefined) {
      unreadable += 1;
      continue;
    }
    const graphs = [workflow.draftGraph];
    if (
      workflow.publishedGraph != null &&
      workflow.publishedGraph !== workflow.draftGraph
    ) {
      graphs.push(workflow.publishedGraph);
    }

    const remainingSkills = input.skills.filter(
      (skill) =>
        !input.targets.some(
          (target) => target.kind === "skill" && target.id === skill.id,
        ),
    );
    let workflowUnreadable = false;
    const nodeTitles: string[] = [];
    for (const rawGraph of graphs) {
      if (!isReadableGraph(rawGraph)) {
        workflowUnreadable = true;
        continue;
      }
      let dependencies: ReturnType<typeof collectWorkflowDependencies>;
      try {
        dependencies = collectWorkflowDependencies(
          parseWorkflowGraph(rawGraph).nodes,
          input.plugins,
          input.skills,
        );
      } catch {
        workflowUnreadable = true;
        continue;
      }
      for (const dependency of dependencies) {
        if (dependency.kind === "mcp") {
          if (
            !input.targets.some(
              (target) => target.kind === "mcp" && target.id === dependency.id,
            )
          )
            continue;
        } else {
          const before = resolveWorkflowSkill(
            dependency.id,
            input.skills,
            input.retiredSkillIds,
          );
          const after = resolveWorkflowSkill(dependency.id, remainingSkills, [
            ...(input.retiredSkillIds ?? []),
            ...input.skills
              .filter((skill) => !remainingSkills.includes(skill))
              .map((skill) => skill.id),
          ]);
          if (before.status === "missing" || after.status !== "missing")
            continue;
        }
        for (const title of dependency.nodeTitles) {
          if (!nodeTitles.includes(title)) nodeTitles.push(title);
        }
      }
    }

    if (workflowUnreadable) {
      unreadable += 1;
    }
    if (nodeTitles.length === 0) continue;
    dependents.push({
      workflowId: workflow.workflowId,
      name: workflow.name,
      nodeTitles,
    });
  }

  return { dependents, unreadable };
}
