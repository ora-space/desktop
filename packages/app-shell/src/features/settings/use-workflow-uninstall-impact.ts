import { useMemo } from "react";
import {
  useWorkflowDrafts,
  useWorkflowLibrary,
} from "../../state/data/workflows";
import { useInstalledPlugins } from "../../state/hooks/use-installed-plugins";
import { useSkills } from "../../state/hooks/use-skills";
import {
  findImportedWorkflowsFromPlugins,
  findWorkflowDependents,
  hasWorkflowSourceCandidate,
  uninstallDependencyTargets,
  type WorkflowDependencyImpact,
  type WorkflowImportedByPlugin,
} from "./workflow-dependency-impact";

/**
 * Outcome of checking whether a pending plugin uninstall breaks existing workflows or leaves
 * imported workflow copies behind.
 *
 * Separating `checking` from `ready` and `failed` lets confirmation dialogs hold the destructive
 * button while a scan or stale background refetch is in flight, without blocking removal when a
 * catalog cannot be read.
 */
export type WorkflowUninstallImpactState =
  | {
      status: "idle";
      checking: false;
      impact: null;
      importedWorkflows: null;
    }
  | {
      status: "checking";
      checking: true;
      scope: "dependencies" | "imported";
      impact: null;
      importedWorkflows: null;
    }
  | {
      status: "failed";
      checking: false;
      impact: null;
      importedWorkflows: null;
      /**
       * The dependency impact stays unknown: the plugin/skill catalogs were unreadable, or the
       * workflow library failed while dependency targets exist.
       */
      dependencyImpactUnknown: boolean;
      /**
       * The retained-copy list stays unknown: the workflow library failed while the uninstall
       * includes a workflow plugin, so "no imported workflows" must never be implied.
       */
      retentionUnknown: boolean;
    }
  | {
      status: "ready";
      checking: false;
      impact: WorkflowDependencyImpact;
      importedWorkflows: WorkflowImportedByPlugin[];
    };

/** Treats initial loads and stale background refetches alike so stale data never reads as final. */
function isQueryRefreshing(query: {
  isPending: boolean;
  isStale: boolean;
  isFetching: boolean;
}): boolean {
  return query.isPending || (query.isStale && query.isFetching);
}

/**
 * Resolves the workflows affected by removing `pluginIds` only while `enabled` is true.
 *
 * Scans both draft and published snapshots once MCP/Skill target identities are known, lists
 * surviving copies imported by workflow plugins without fetching individual drafts, and isolates
 * single draft fetch failures into the unreadable count instead of discarding confirmed dependents.
 */
export function useWorkflowUninstallImpact(
  pluginIds: readonly string[],
  enabled: boolean,
): WorkflowUninstallImpactState {
  const active = enabled && pluginIds.length > 0;
  const pluginIdsKey = pluginIds.join("\0");
  const stablePluginIds = useMemo(
    () => (pluginIdsKey === "" ? [] : pluginIdsKey.split("\0")),
    [pluginIdsKey],
  );

  const plugins = useInstalledPlugins(active);
  const needsSkillCatalog =
    plugins.data === undefined ||
    stablePluginIds.some((pluginId) => {
      const plugin = plugins.data.find(
        (candidate) => candidate.id === pluginId,
      );
      return plugin === undefined || plugin.kind === "skill";
    });
  const skills = useSkills(active && needsSkillCatalog);
  const skillsRefreshing =
    active && needsSkillCatalog && isQueryRefreshing(skills);
  const skillsFailed =
    active && needsSkillCatalog && !skillsRefreshing && skills.isError;
  const catalogsRefreshing =
    active && (isQueryRefreshing(plugins) || skillsRefreshing);
  const catalogsFailed =
    active && !catalogsRefreshing && (plugins.isError || skillsFailed);

  const targets = useMemo(() => {
    if (!active || catalogsRefreshing || catalogsFailed) return [];
    return uninstallDependencyTargets(
      stablePluginIds,
      plugins.data ?? [],
      skills.data ?? [],
    );
  }, [
    active,
    catalogsRefreshing,
    catalogsFailed,
    stablePluginIds,
    plugins.data,
    skills.data,
  ]);

  const workflowSourceCandidate =
    active &&
    !catalogsRefreshing &&
    !catalogsFailed &&
    hasWorkflowSourceCandidate(stablePluginIds, plugins.data ?? []);

  const catalogsSettledEmpty =
    active &&
    !catalogsRefreshing &&
    !catalogsFailed &&
    targets.length === 0 &&
    !workflowSourceCandidate;
  const libraryEnabled = active && !catalogsFailed && !catalogsSettledEmpty;
  const library = useWorkflowLibrary(libraryEnabled);
  const libraryRefreshing = libraryEnabled && isQueryRefreshing(library);
  const libraryFailed = libraryEnabled && !libraryRefreshing && library.isError;

  const shouldScanDrafts =
    active &&
    !catalogsRefreshing &&
    !catalogsFailed &&
    targets.length > 0 &&
    !libraryRefreshing &&
    !libraryFailed;

  const workflowIds = useMemo(
    () =>
      shouldScanDrafts
        ? (library.data ?? []).map((workflow) => workflow.id)
        : [],
    [shouldScanDrafts, library.data],
  );
  const drafts = useWorkflowDrafts(workflowIds);
  const draftsRefreshing =
    shouldScanDrafts && drafts.some((draft) => isQueryRefreshing(draft));

  const checking =
    active && (catalogsRefreshing || libraryRefreshing || draftsRefreshing);
  const failed = active && !checking && (catalogsFailed || libraryFailed);

  const draftsVersion = shouldScanDrafts
    ? drafts
        .map(
          (draft, index) =>
            `${workflowIds[index] ?? ""}:${draft?.dataUpdatedAt ?? 0}:${draft?.errorUpdatedAt ?? 0}:${draft?.isError ? 1 : 0}:${draft?.data?.draft.updatedAt ?? ""}:${draft?.data?.published?.id ?? ""}`,
        )
        .join("|")
    : "";

  const draftGraphs = useMemo(
    () =>
      shouldScanDrafts
        ? drafts.map((draft) => ({
            draftGraph: draft?.isError ? undefined : draft?.data?.draft.graph,
            publishedGraph: draft?.isError
              ? undefined
              : draft?.data?.published?.graph,
          }))
        : [],
    // `useQueries` returns a fresh array reference on every render; `draftsVersion` tracks every
    // slot's workflow id, error state, and update timestamps so graph parsing only runs on changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [draftsVersion, shouldScanDrafts],
  );

  const impact = useMemo(() => {
    if (!active || checking || failed) return null;
    if (targets.length === 0) {
      return { dependents: [], unreadable: 0 };
    }
    const workflows = (library.data ?? []).map((summary, index) => ({
      workflowId: summary.id,
      name: summary.name,
      draftGraph: draftGraphs[index]?.draftGraph,
      publishedGraph: draftGraphs[index]?.publishedGraph,
    }));
    return findWorkflowDependents({
      workflows,
      plugins: plugins.data ?? [],
      skills: skills.data ?? [],
      targets,
    });
  }, [
    active,
    checking,
    failed,
    targets,
    library.data,
    draftGraphs,
    plugins.data,
    skills.data,
  ]);

  const importedWorkflows = useMemo(() => {
    if (!active || checking || failed || !workflowSourceCandidate) return [];
    return findImportedWorkflowsFromPlugins(
      library.data ?? [],
      stablePluginIds,
    );
  }, [
    active,
    checking,
    failed,
    workflowSourceCandidate,
    library.data,
    stablePluginIds,
  ]);

  if (!active) {
    return {
      status: "idle",
      checking: false,
      impact: null,
      importedWorkflows: null,
    };
  }
  if (checking) {
    return {
      status: "checking",
      checking: true,
      scope:
        targets.length === 0 && workflowSourceCandidate
          ? "imported"
          : "dependencies",
      impact: null,
      importedWorkflows: null,
    };
  }
  if (failed || impact === null) {
    return {
      status: "failed",
      checking: false,
      impact: null,
      importedWorkflows: null,
      dependencyImpactUnknown:
        catalogsFailed || (libraryFailed && targets.length > 0),
      retentionUnknown: libraryFailed && workflowSourceCandidate,
    };
  }
  return {
    status: "ready",
    checking: false,
    impact,
    importedWorkflows,
  };
}
