import type {
  ImportedWorkflowOutcome,
  InstalledPlugin,
  MarketplaceWorkflowOutcome,
  Skill,
} from "@ora/contracts";
import { parseWorkflowGraph } from "@ora/workflow-runtime";
import {
  collectWorkflowDependencies,
  type WorkflowDependency,
} from "../workflow-editor/workflow-transfer";

/** One workflow document the imported package created, with the plugin references it carries. */
export interface ImportedWorkflowReport {
  workflowId: string;
  name: string;
  version: string;
  /**
   * Local readiness of every MCP and Skill binding in the graph, missing entries first; `null`
   * when the created workflow could not be read back, so the report says so instead of claiming
   * the workflow needs nothing.
   */
  dependencies: WorkflowDependency[] | null;
}

/** One document the import refused, named by the package-relative path it was read from. */
export interface RefusedWorkflowReport {
  sourceFile: string;
  reason: string;
}

/** How many distinct plugin references of one readiness state the import brought in. */
export interface WorkflowImportDependencyCounts {
  installed: number;
  unavailable: number;
  missing: number;
}

/** Everything the import result surface presents for one local `.orax` package. */
export interface WorkflowImportReport {
  /** File name of the archive the user chose, shown under the dialog title. */
  fileName: string;
  /** Workflows the import created, in package order. */
  workflows: ImportedWorkflowReport[];
  /** Documents the import refused, in package order. */
  refused: RefusedWorkflowReport[];
  /**
   * Distinct references by readiness across every readable workflow. A plugin bound by several
   * workflows counts once: the summary answers "what is missing on this machine", not "how many
   * bindings did the package carry".
   */
  counts: WorkflowImportDependencyCounts;
}

/**
 * Extracts the archive's own file name from the path the host reported.
 *
 * The native picker returns an OS path while the same import can be reached with either
 * separator, so the name is taken from the last separator of either kind.
 */
export function importedArchiveName(path: string): string {
  const segment = path.split(/[\\/]/).pop() ?? path;
  return segment === "" ? path : segment;
}

/**
 * Reduces a marketplace document report to the local import outcomes the report dialog renders.
 *
 * The dialog lists the documents that became workflows and the ones that were refused; the skips a
 * marketplace upgrade reports are counts rather than rows, so they stay in the toast summary shown
 * beside it. Keeping one report surface for both entry points means a user reads the same
 * vocabulary whichever path delivered the document.
 */
export function marketplaceReportOutcomes(
  outcomes: readonly MarketplaceWorkflowOutcome[],
): ImportedWorkflowOutcome[] {
  const converted: ImportedWorkflowOutcome[] = [];
  for (const outcome of outcomes) {
    switch (outcome.state) {
      case "imported":
        converted.push({
          state: "imported",
          sourceFile: outcome.sourceFile,
          workflowId: outcome.workflowId,
          name: outcome.name,
          version: outcome.version,
        });
        break;
      case "failed":
        converted.push({
          state: "failed",
          sourceFile: outcome.sourceFile,
          reason: outcome.reason,
        });
        break;
      case "skipped_processed":
      case "skipped_conflict":
        break;
    }
  }
  return converted;
}

/**
 * Resolves every workflow an imported package created against the local plugin and Skill
 * catalogs, using the same rules as the editor's import preview.
 *
 * The backend reports one outcome per document and stays free of MCP/Skill identity, so the graph
 * is read back per created workflow (`graphs`) and handed to the one dependency resolver both
 * import entries share. A graph that could not be read keeps its slot with `dependencies: null`
 * rather than dropping the workflow from the report.
 */
export function buildWorkflowImportReport(input: {
  fileName: string;
  outcomes: readonly ImportedWorkflowOutcome[];
  /** Graph JSON of each created workflow, keyed by the workflow id the import returned. */
  graphs: ReadonlyMap<string, string>;
  plugins: readonly InstalledPlugin[];
  skills: readonly Skill[];
}): WorkflowImportReport {
  const workflows: ImportedWorkflowReport[] = [];
  const refused: RefusedWorkflowReport[] = [];
  /** Distinct references seen across workflows, so one missing plugin is reported once. */
  const counted = new Map<string, WorkflowDependency["status"]>();

  for (const outcome of input.outcomes) {
    if (outcome.state === "failed") {
      refused.push({ sourceFile: outcome.sourceFile, reason: outcome.reason });
      continue;
    }
    const graph = input.graphs.get(outcome.workflowId);
    if (graph === undefined) {
      workflows.push({
        workflowId: outcome.workflowId,
        name: outcome.name,
        version: outcome.version,
        dependencies: null,
      });
      continue;
    }
    const dependencies = collectWorkflowDependencies(
      parseWorkflowGraph(graph).nodes,
      input.plugins,
      input.skills,
    );
    for (const dependency of dependencies) {
      counted.set(`${dependency.kind}:${dependency.id}`, dependency.status);
    }
    workflows.push({
      workflowId: outcome.workflowId,
      name: outcome.name,
      version: outcome.version,
      dependencies,
    });
  }

  const counts: WorkflowImportDependencyCounts = {
    installed: 0,
    unavailable: 0,
    missing: 0,
  };
  for (const status of counted.values()) counts[status] += 1;

  return { fileName: input.fileName, workflows, refused, counts };
}
