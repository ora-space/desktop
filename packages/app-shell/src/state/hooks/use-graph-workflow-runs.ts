import { useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { DemoWorkflow } from "@ora/workflow-mock";
import { useWorkflowRuntime } from "../../features/workflow-run/runtime/workflow-runtime-context";
import type {
  GraphWorkflowRun,
  WorkflowArtifact,
} from "../../features/workflow-run/runtime/types";
import { useWorkspaceSelectionStore } from "../stores/workspace-selection-store";
import { queryKeys } from "./query-keys";

/**
 * Keeps react-query run caches in sync with mock-engine mutations
 * so sidebar status dots update without a Theater UI yet.
 */
export function useGraphWorkflowRunLiveSync() {
  const runtime = useWorkflowRuntime();
  const queryClient = useQueryClient();
  useEffect(() => {
    return runtime.runs.watch((run) => {
      const clone = structuredClone(run);
      queryClient.setQueryData(queryKeys.workflowRun(run.id), clone);
      // Patch the project list in place to avoid refetch flicker on every node tick.
      queryClient.setQueryData(
        queryKeys.workflowRuns(run.projectId),
        (previous: GraphWorkflowRun[] | undefined) => {
          if (previous === undefined) {
            return previous;
          }
          const index = previous.findIndex((item) => item.id === run.id);
          if (index < 0) {
            return [clone, ...previous];
          }
          const next = previous.slice();
          next[index] = clone;
          return next;
        },
      );
    });
  }, [runtime, queryClient]);
}

/** Lists graph workflow runs for a project (D1: react-query list). */
export function useGraphWorkflowRuns(projectId: string | null | undefined) {
  const runtime = useWorkflowRuntime();
  return useQuery({
    queryKey: queryKeys.workflowRuns(projectId ?? ""),
    queryFn: () => runtime.runs.list(projectId!),
    enabled: projectId != null && projectId !== "",
  });
}

/** Loads one graph workflow run by id. */
export function useGraphWorkflowRun(runId: string | null | undefined) {
  const runtime = useWorkflowRuntime();
  return useQuery({
    queryKey: queryKeys.workflowRun(runId ?? ""),
    queryFn: () => runtime.runs.get(runId!),
    enabled: runId != null && runId !== "",
  });
}

/** Deploys (registers + mounts) a definition onto a project. */
export function useMountWorkflow() {
  const runtime = useWorkflowRuntime();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      projectId,
      definition,
    }: {
      projectId: string;
      definition: DemoWorkflow;
    }) => runtime.host.mount(projectId, definition),
    onSuccess: (_mount, variables) => {
      void queryClient.invalidateQueries({
        queryKey: queryKeys.workflowMounts(variables.projectId),
      });
      void queryClient.invalidateQueries({
        queryKey: queryKeys.workflowMountsByDefinition(variables.definition.id),
      });
    },
  });
}

/** Starts a graph workflow run from an already-mounted definition. */
export function useCreateGraphWorkflowRun() {
  const runtime = useWorkflowRuntime();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: {
      projectId: string;
      definitionId: string;
      kickoffInput?: string;
    }) => runtime.runs.create(input),
    onSuccess: (run) => {
      void queryClient.invalidateQueries({
        queryKey: queryKeys.workflowRuns(run.projectId),
      });
      queryClient.setQueryData(queryKeys.workflowRun(run.id), run);
    },
  });
}

/** Deletes a graph workflow run (cancels first when still active). */
export function useDeleteGraphWorkflowRun() {
  const runtime = useWorkflowRuntime();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async ({
      runId,
      projectId,
    }: {
      runId: string;
      projectId: string;
    }) => {
      await runtime.runs.delete(runId);
      return { runId, projectId };
    },
    onSuccess: ({ runId, projectId }) => {
      void queryClient.invalidateQueries({
        queryKey: queryKeys.workflowRuns(projectId),
      });
      queryClient.removeQueries({ queryKey: queryKeys.workflowRun(runId) });
      queryClient.removeQueries({ queryKey: queryKeys.workflowArtifacts(runId) });
      const selection = useWorkspaceSelectionStore.getState().selection;
      if (selection.workflowRunId === runId) {
        useWorkspaceSelectionStore.getState().clearWorkflowRunSelection(projectId);
      }
    },
  });
}

/** Starts a pending graph workflow run (no-op if already past pending). */
export function useStartGraphWorkflowRun() {
  const runtime = useWorkflowRuntime();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async ({
      runId,
      projectId,
    }: {
      runId: string;
      projectId: string;
    }) => {
      await runtime.runs.start(runId);
      return { runId, projectId };
    },
    onSuccess: async ({ runId, projectId }) => {
      const run = await runtime.runs.get(runId);
      if (run !== null) {
        queryClient.setQueryData(queryKeys.workflowRun(runId), run);
        queryClient.setQueryData(
          queryKeys.workflowRuns(projectId),
          (previous: GraphWorkflowRun[] | undefined) => {
            if (previous === undefined) {
              return previous;
            }
            return previous.map((item) => (item.id === runId ? run : item));
          },
        );
      }
    },
  });
}

/** Cancels an in-flight graph workflow run without deleting it. */
export function useCancelGraphWorkflowRun() {
  const runtime = useWorkflowRuntime();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async ({
      runId,
      projectId,
    }: {
      runId: string;
      projectId: string;
    }) => {
      await runtime.runs.cancel(runId);
      return { runId, projectId };
    },
    onSuccess: async ({ runId, projectId }) => {
      const run = await runtime.runs.get(runId);
      if (run !== null) {
        queryClient.setQueryData(queryKeys.workflowRun(runId), run);
        queryClient.setQueryData(
          queryKeys.workflowRuns(projectId),
          (previous: GraphWorkflowRun[] | undefined) => {
            if (previous === undefined) {
              return previous;
            }
            return previous.map((item) => (item.id === runId ? run : item));
          },
        );
      }
    },
  });
}

/**
 * Creates a fresh pending run from a finished one, starts it, and returns the new record.
 * Mirrors Settings “Run again”: history stays on the old row; execution continues on a sibling.
 */
export function useRerunGraphWorkflowRun() {
  const runtime = useWorkflowRuntime();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (source: GraphWorkflowRun) => {
      const created = await runtime.runs.create({
        projectId: source.projectId,
        definitionId: source.definitionId,
        kickoffInput: source.kickoffInput,
      });
      await runtime.runs.start(created.id);
      const started = await runtime.runs.get(created.id);
      return started ?? created;
    },
    onSuccess: (run) => {
      void queryClient.invalidateQueries({
        queryKey: queryKeys.workflowRuns(run.projectId),
      });
      queryClient.setQueryData(queryKeys.workflowRun(run.id), run);
    },
  });
}

/** Renames a graph workflow run for sidebar / workspace labeling. */
export function useRenameGraphWorkflowRun() {
  const runtime = useWorkflowRuntime();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      runId,
      name,
    }: {
      runId: string;
      name: string;
    }) => runtime.runs.rename(runId, name),
    onSuccess: (run) => {
      void queryClient.invalidateQueries({
        queryKey: queryKeys.workflowRuns(run.projectId),
      });
      queryClient.setQueryData(queryKeys.workflowRun(run.id), run);
    },
  });
}

/**
 * Pending-only: patch instruction/description on the run snapshot node copy.
 * Does not mutate the mounted library definition.
 */
export function useUpdateGraphWorkflowRunSnapshotNode() {
  const runtime = useWorkflowRuntime();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      runId,
      nodeId,
      patch,
    }: {
      runId: string;
      nodeId: string;
      patch: {
        instruction?: string;
        description?: string;
      };
    }) => runtime.runs.updateSnapshotNode(runId, nodeId, patch),
    onSuccess: (run) => {
      void queryClient.invalidateQueries({
        queryKey: queryKeys.workflowRuns(run.projectId),
      });
      queryClient.setQueryData(queryKeys.workflowRun(run.id), run);
    },
  });
}

/**
 * Lists artifacts for a run and patches the cache on `artifact_added`
 * so Theater act cards update without refetching on every node tick.
 */
export function useGraphWorkflowArtifacts(runId: string | null | undefined) {
  const runtime = useWorkflowRuntime();
  const queryClient = useQueryClient();
  const [revealedId, setRevealedId] = useState<string | null>(null);

  const query = useQuery({
    queryKey: queryKeys.workflowArtifacts(runId ?? ""),
    queryFn: () => runtime.runs.listArtifacts(runId!),
    enabled: runId != null && runId !== "",
  });

  useEffect(() => {
    setRevealedId(null);
  }, [runId]);

  useEffect(() => {
    if (runId == null || runId === "") {
      return;
    }
    return runtime.runs.subscribe(runId, (event) => {
      if (event.type !== "artifact_added") {
        return;
      }
      const artifact = structuredClone(event.artifact);
      queryClient.setQueryData(
        queryKeys.workflowArtifacts(runId),
        (previous: WorkflowArtifact[] | undefined) => {
          if (previous === undefined) {
            return [artifact];
          }
          if (previous.some((item) => item.id === artifact.id)) {
            return previous;
          }
          return [...previous, artifact];
        },
      );
      setRevealedId(artifact.id);
    });
  }, [runtime, queryClient, runId]);

  return {
    ...query,
    artifacts: query.data ?? [],
    revealedId,
  };
}
