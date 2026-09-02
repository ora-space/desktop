import { useMemo } from "react";
import { useQueries } from "@tanstack/react-query";
import type { WorkflowAgentModel } from "@ora/workflow-mock";
import { useContractsClient } from "../../contracts-client-context";
import type { AgentEntry } from "../../features/chat/agent-catalog";
import { useAvailableAgents } from "./use-available-agents";
import { useWorkspaceSelectionStore } from "../stores/workspace-selection-store";
import { queryKeys } from "./query-keys";
import { useProjects } from "./use-projects";
import { useTasks } from "./use-tasks";
import { useWorkspaces } from "./use-workspaces";

/** Per-agent discovery state so the inspector can show a spinner or retry per row. */
export interface WorkflowAgentCliStatus {
  isLoading: boolean;
  isError: boolean;
}

/** Discovered agent × model pairs for workflow node configuration. */
export interface WorkflowAgentModelsCatalog {
  /** The agents this installation can offer, in the order the picker lists them. */
  agents: AgentEntry[];
  agentModels: WorkflowAgentModel[];
  /** Models grouped by agent, mirroring the two-section picker in chat. */
  modelsByCli: ReadonlyMap<string, WorkflowAgentModel[]>;
  /** Loading/error state for every offered agent, keyed by its identity. */
  cliStatus: Readonly<Record<string, WorkflowAgentCliStatus>>;
  isLoading: boolean;
  isError: boolean;
  refetch: () => void;
}

/**
 * Discovers real models through each agent plugin
 * and flattens them into the workflow inspector's single picker list.
 *
 * Discovery requires a cwd-bearing workspace. Preference order matches chat surfaces,
 * then falls back to the first listed project so Settings can still discover
 * models when no workspace selection is active.
 */
export function useWorkflowAgentModels(): WorkflowAgentModelsCatalog {
  const client = useContractsClient();
  const agents = useAvailableAgents();
  const selection = useWorkspaceSelectionStore((state) => state.selection);
  const projectsQuery = useProjects();
  const tasksQuery = useTasks();
  const workspacesQuery = useWorkspaces();
  const workspaceId = discoveryWorkspaceId(
    selection,
    projectsQuery.data?.[0]?.id ?? null,
    tasksQuery.data ?? [],
    workspacesQuery.data ?? [],
  );
  const projectsPending =
    projectsQuery.isPending &&
    selection.projectId === null &&
    selection.taskId === null;

  const modelQueries = useQueries({
    queries: agents.map((agent) => ({
      queryKey: queryKeys.agentModels(agent.agentRef, workspaceId),
      enabled: workspaceId !== null,
      queryFn: () =>
        client.agentRuntime.listModels({
          workspaceId: workspaceId!,
          agentRef: agent.agentRef,
        }),
      retry: false,
    })),
  });

  const agentModels = useMemo(() => {
    // Derive from the same offered-agent list the chat picker renders, so an agent
    // installed while Ora runs is discovered here too without a second list to sync.
    const models: WorkflowAgentModel[] = [];
    for (const [index, agent] of agents.entries()) {
      const discovered = modelQueries[index]?.data?.models ?? [];
      for (const model of discovered) {
        models.push({
          agentCli: agent.agentRef,
          modelId: model.id,
          label: `${agent.label} · ${model.displayName}`,
        });
      }
    }
    return models;
  }, [agents, modelQueries]);

  const modelsByCli = useMemo(() => {
    const byCli = new Map<string, WorkflowAgentModel[]>();
    for (const model of agentModels) {
      const existing = byCli.get(model.agentCli);
      if (existing === undefined) {
        byCli.set(model.agentCli, [model]);
      } else {
        existing.push(model);
      }
    }
    return byCli;
  }, [agentModels]);

  const cliStatus = useMemo(
    () =>
      Object.fromEntries(
        agents.map((agent, index) => {
          const query = modelQueries[index];
          return [
            agent.agentRef,
            {
              isLoading: query?.isPending === true,
              isError: query?.isError === true,
            },
          ];
        }),
      ),
    [agents, modelQueries],
  );

  const isLoading =
    projectsPending ||
    (selection.taskId !== null && tasksQuery.isPending) ||
    (selection.taskId === null &&
      selection.projectId !== null &&
      workspacesQuery.isPending) ||
    (workspaceId !== null &&
      modelQueries.some((query) => query.isPending || query.isFetching));
  const isError =
    workspaceId !== null &&
    !isLoading &&
    modelQueries.every((query) => query.isError) &&
    agentModels.length === 0;

  return {
    agents,
    agentModels,
    modelsByCli,
    cliStatus,
    isLoading,
    isError,
    refetch: () => {
      void projectsQuery.refetch();
      void tasksQuery.refetch();
      void workspacesQuery.refetch();
      for (const query of modelQueries) {
        void query.refetch();
      }
    },
  };
}

/** Picks the cwd target used to discover models for the workflow editor. */
function discoveryWorkspaceId(
  selection: { projectId: string | null; taskId: string | null },
  fallbackProjectId: string | null,
  tasks: readonly { id: string; workspaceId: string }[],
  workspaces: readonly {
    id: string;
    projectId: string;
    kind: "main" | "isolated";
  }[],
): string | null {
  if (selection.taskId !== null) {
    const task = tasks.find((candidate) => candidate.id === selection.taskId);
    return task === undefined ? null : task.workspaceId;
  }
  if (selection.projectId !== null) {
    const workspace = workspaces.find(
      (candidate) =>
        candidate.projectId === selection.projectId &&
        candidate.kind === "main",
    );
    return workspace === undefined ? null : workspace.id;
  }
  if (fallbackProjectId !== null) {
    const workspace = workspaces.find(
      (candidate) =>
        candidate.projectId === fallbackProjectId && candidate.kind === "main",
    );
    return workspace === undefined ? null : workspace.id;
  }
  return null;
}
