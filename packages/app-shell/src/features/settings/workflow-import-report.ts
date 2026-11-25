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
import type { MarketplaceImportTrigger } from "./plugin-install-feedback";

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

/** One document the operation left unimported because a workflow already uses its name. */
export interface UnresolvedWorkflowEntry {
  /** Canonical identifier of the plugin or Pack member that carried the document. */
  pluginId: string;
  /** Package-relative path the document was read from. */
  sourceFile: string;
  /** Name of the surviving workflow in the library that blocks this document's import. */
  name: string;
  /**
   * Whether the record already carried this conflict before the operation ran. A known conflict was
   * reported once; an upgrade lists it as still unresolved instead of opening the report for it.
   */
  knownConflict: boolean;
}

/** One document whose previously imported copy was deleted during the running install cycle. */
export interface DeletedWorkflowCopyEntry {
  /** Canonical identifier of the plugin or Pack member that carried the document. */
  pluginId: string;
  /** Package-relative path whose copy was deleted. */
  sourceFile: string;
}

/** The outcomes one import returned, plus the archive or plugin name to show for them. */
export interface WorkflowImportReportInput {
  fileName: string;
  outcomes: readonly ImportedWorkflowOutcome[];
  /**
   * Documents the operation left unimported because of a name conflict, listed under the report's
   * "not imported" group with the recovery the user can take.
   */
  unresolved?: readonly UnresolvedWorkflowEntry[];
  /**
   * Documents whose copy the user deleted during the running install cycle. They are not a failure
   * and not something to retry from the card, so the report names them with their own recovery.
   */
  deletedCopies?: readonly DeletedWorkflowCopyEntry[];
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
 * The dialog lists the documents that became workflows and the ones that were refused; the skipped
 * documents keep their own groups on the marketplace surface, because a conflict names the workflow
 * that blocked it and a deleted copy names the user's own earlier choice. Keeping one report
 * surface for both entry points means a user reads the same vocabulary whichever path delivered the
 * document.
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
      case "skipped_deleted_copy":
      case "skipped_conflict":
        break;
    }
  }
  return converted;
}

/**
 * Builds the report dialog input for a marketplace install, upgrade, or retry response, or returns
 * `null` when the response has nothing the user has to act on.
 *
 * What opens the dialog follows the operation: an install or a card retry opens it whenever a
 * document is left unimported by a name conflict, because the user asked for those documents now;
 * an upgrade opens it only for a conflict it is reporting for the first time, and keeps a known
 * conflict in its neutral summary otherwise. Whenever the dialog does open, every conflict of the
 * batch is listed, with the known ones marked as still unresolved.
 */
export function marketplaceWorkflowReportInput(
  pluginId: string,
  workflows: readonly MarketplaceWorkflowOutcome[],
  trigger: MarketplaceImportTrigger,
): WorkflowImportReportInput | null {
  const outcomes = marketplaceReportOutcomes(workflows);
  const unresolved = workflows.flatMap((outcome) =>
    outcome.state === "skipped_conflict"
      ? [
          {
            pluginId: outcome.pluginId,
            sourceFile: outcome.sourceFile,
            name: outcome.name,
            knownConflict: outcome.knownConflict,
          },
        ]
      : [],
  );
  const opensForConflict =
    trigger === "update"
      ? unresolved.some((entry) => !entry.knownConflict)
      : unresolved.length > 0;
  // An upgrade keeps deleted copies in its neutral summary only, even when the report opens for
  // another reason; card retries still list them with the reinstall recovery hint.
  const deletedCopies =
    trigger === "update"
      ? []
      : workflows.flatMap((outcome) =>
          outcome.state === "skipped_deleted_copy"
            ? [{ pluginId: outcome.pluginId, sourceFile: outcome.sourceFile }]
            : [],
        );
  if (outcomes.length === 0 && !opensForConflict) return null;
  return {
    fileName: pluginId,
    outcomes,
    unresolved,
    deletedCopies,
  };
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
