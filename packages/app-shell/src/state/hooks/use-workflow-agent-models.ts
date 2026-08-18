import { useMemo } from "react";
import { useQueries } from "@tanstack/react-query";
import type { WarmSessionTarget } from "@ora/contracts";
import type { KnownAgentCli } from "../../features/chat/model-catalog";
import { findModelOption, selectableValues } from "@ora/chat";
import type { WorkflowAgentModel } from "@ora/workflow-mock";
import { useContractsClient } from "../../contracts-client-context";
import {
  AGENT_CLI_LABELS,
  AGENT_CLI_ORDER,
} from "../../features/chat/model-catalog";
import { useWorkspaceSelectionStore } from "../stores/workspace-selection-store";
import { queryKeys } from "./query-keys";
import { useProjects } from "./use-projects";

/** Per-CLI discovery state so the inspector can show a spinner or retry per row. */
export interface WorkflowAgentCliStatus {
  isLoading: boolean;
  isError: boolean;
}

/** Discovered Agent CLI × model pairs for workflow node configuration. */
export interface WorkflowAgentModelsCatalog {
  agentModels: WorkflowAgentModel[];
  /** Models grouped by CLI, mirroring the two-section picker in chat. */
  modelsByCli: ReadonlyMap<KnownAgentCli, WorkflowAgentModel[]>;
  /** Loading/error state for every configured CLI, keyed by CLI. */
  cliStatus: Readonly<Record<KnownAgentCli, WorkflowAgentCliStatus>>;
  isLoading: boolean;
  isError: boolean;
  refetch: () => void;
}

/**
 * Discovers real models the same way chat does — by warming each Agent CLI —
 * and flattens them into the workflow inspector's single picker list.
 *
 * Warm requires a cwd-bearing target. Preference order matches chat surfaces,
 * then falls back to the first listed project so Settings can still discover
 * models when no workspace selection is active.
 */
export function useWorkflowAgentModels(): WorkflowAgentModelsCatalog {
  const client = useContractsClient();
  const selection = useWorkspaceSelectionStore((state) => state.selection);
  const projectsQuery = useProjects();
  const target = discoveryTarget(
    selection,
    projectsQuery.data?.[0]?.id ?? null,
  );
  const projectsPending =
    projectsQuery.isPending &&
    selection.projectId === null &&
    selection.taskId === null;

  const warmQueries = useQueries({
    queries: AGENT_CLI_ORDER.map((agentCli) => ({
      queryKey: queryKeys.warmSession(target, agentCli),
      enabled: target !== null,
      queryFn: () =>
        client.session.warm({
          target: target!,
          agentRef: agentCli,
        }),
      staleTime: Infinity,
      gcTime: Infinity,
      retry: false,
    })),
  });

  const agentModels = useMemo(() => {
    // Derive from AGENT_CLI_ORDER, the same single list the chat picker renders,
    // so a CLI added there is discovered here too without a second list to sync.
    const models: WorkflowAgentModel[] = [];
    for (const [index, agentCli] of AGENT_CLI_ORDER.entries()) {
      const options = warmQueries[index]?.data?.configOptions;
      if (options === undefined) {
        continue;
      }
      const modelOption = findModelOption(options);
      if (modelOption === null) {
        continue;
      }
      for (const value of selectableValues(modelOption)) {
        models.push({
          agentCli,
          modelId: value.value,
          label: `${AGENT_CLI_LABELS[agentCli]} · ${value.name}`,
        });
      }
    }
    return models;
  }, [warmQueries]);

  const modelsByCli = useMemo(() => {
    const byCli = new Map<KnownAgentCli, WorkflowAgentModel[]>();
    for (const model of agentModels) {
      const cli = model.agentCli as KnownAgentCli;
      const existing = byCli.get(cli);
      if (existing === undefined) {
        byCli.set(cli, [model]);
      } else {
        existing.push(model);
      }
    }
    return byCli;
  }, [agentModels]);

  const cliStatus = useMemo(
    () =>
      Object.fromEntries(
        AGENT_CLI_ORDER.map((agentCli, index) => {
          const query = warmQueries[index];
          return [
            agentCli,
            {
              isLoading: query?.isPending === true,
              isError: query?.isError === true,
            },
          ];
        }),
      ) as Readonly<Record<KnownAgentCli, WorkflowAgentCliStatus>>,
    [warmQueries],
  );

  const isLoading =
    projectsPending ||
    (target !== null &&
      warmQueries.some((query) => query.isPending || query.isFetching));
  const isError =
    target !== null &&
    !isLoading &&
    warmQueries.every((query) => query.isError) &&
    agentModels.length === 0;

  return {
    agentModels,
    modelsByCli,
    cliStatus,
    isLoading,
    isError,
    refetch: () => {
      void projectsQuery.refetch();
      for (const query of warmQueries) {
        void query.refetch();
      }
    },
  };
}

/** Picks the cwd target used to discover models for the workflow editor. */
function discoveryTarget(
  selection: { projectId: string | null; taskId: string | null },
  fallbackProjectId: string | null,
): WarmSessionTarget | null {
  if (selection.taskId !== null) {
    return { type: "task", taskId: selection.taskId };
  }
  if (selection.projectId !== null) {
    return { type: "projectRoot", projectId: selection.projectId };
  }
  if (fallbackProjectId !== null) {
    return { type: "projectRoot", projectId: fallbackProjectId };
  }
  return null;
}
