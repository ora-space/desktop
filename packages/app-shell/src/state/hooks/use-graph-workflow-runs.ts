import { useEffect, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useWorkflowRuntime } from "../../features/workflow-run/workflow-runtime-context";
import type {
  GraphWorkflowRun,
  HitlRequest,
  WorkflowDefinitionInput,
  WorkflowRunLiveSnapshot,
} from "@ora/workflow-runtime";
import { normalizeWorkflowDefinition } from "@ora/workflow-runtime";
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
      definition: WorkflowDefinitionInput;
    }) => runtime.host.mount(projectId, normalizeWorkflowDefinition(definition)),
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
    mutationFn: async ({ runId }: { runId: string }) => {
      const run = await runtime.runs.start(runId);
      return run;
    },
    onSuccess: (run) => {
      queryClient.setQueryData(queryKeys.workflowRun(run.id), run);
      queryClient.setQueryData(
        queryKeys.workflowRuns(run.projectId),
        (previous: GraphWorkflowRun[] | undefined) => {
          if (previous === undefined) {
            return previous;
          }
          return previous.map((item) => (item.id === run.id ? run : item));
        },
      );
    },
  });
}

/** Cancels an in-flight graph workflow run without deleting it. */
export function useCancelGraphWorkflowRun() {
  const runtime = useWorkflowRuntime();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async ({ runId }: { runId: string }) => {
      const run = await runtime.runs.cancel(runId);
      return run;
    },
    onSuccess: (run) => {
      queryClient.setQueryData(queryKeys.workflowRun(run.id), run);
      queryClient.setQueryData(
        queryKeys.workflowRuns(run.projectId),
        (previous: GraphWorkflowRun[] | undefined) => {
          if (previous === undefined) {
            return previous;
          }
          return previous.map((item) => (item.id === run.id ? run : item));
        },
      );
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
      return runtime.runs.start(created.id);
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

/** Submits an open HITL request and resumes the mock run. */
export function useSubmitGraphWorkflowHitl() {
  const runtime = useWorkflowRuntime();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      runId,
      requestId,
      payload,
    }: {
      runId: string;
      requestId: string;
      payload: Record<string, unknown>;
    }) => runtime.runs.submitHitl(runId, requestId, payload),
    onSuccess: (run) => {
      queryClient.setQueryData(queryKeys.workflowRun(run.id), run);
    },
  });
}

/**
 * Live artifacts for a run on a single `subscribe`.
 * Optional handlers piggy-back the same stream (HITL toast / finish) so the
 * workspace does not open a second subscription.
 */
export function useGraphWorkflowRunLive(
  runId: string | null | undefined,
  handlers: {
    onHitlRequired?: (request: HitlRequest) => void;
    onRunFinished?: () => void;
  } = {},
) {
  const runtime = useWorkflowRuntime();
  const queryClient = useQueryClient();
  const [revealedId, setRevealedId] = useState<string | null>(null);
  const handlersRef = useRef(handlers);
  handlersRef.current = handlers;

  const query = useQuery({
    queryKey: queryKeys.workflowArtifacts(runId ?? ""),
    queryFn: () => runtime.runs.getLiveSnapshot(runId!),
    enabled: runId != null && runId !== "",
    // Once loaded, the cursor stream owns freshness. Automatic refetch could
    // overwrite an event applied after the server produced its snapshot.
    staleTime: Number.POSITIVE_INFINITY,
  });
  const snapshotRef = useRef(query.data);
  snapshotRef.current = query.data;
  const hasSnapshot = query.data !== undefined && query.data !== null;

  useEffect(() => {
    setRevealedId(null);
  }, [runId]);

  useEffect(() => {
    if (runId == null || runId === "" || !hasSnapshot) {
      return;
    }
    return runtime.runs.subscribe(runId, (event) => {
      const cacheKey = queryKeys.workflowArtifacts(runId);
      if (event.type === "artifact_added") {
        const artifact = structuredClone(event.artifact);
        queryClient.setQueryData(
          cacheKey,
          (previous: WorkflowRunLiveSnapshot | null | undefined) => {
            if (previous === undefined || previous === null) {
              return previous;
            }
            if (previous.artifacts.some((item) => item.id === artifact.id)) {
              return previous;
            }
            return {
              ...previous,
              artifacts: [...previous.artifacts, artifact],
              cursor: event.cursor,
            };
          },
        );
        setRevealedId(artifact.id);
        return;
      }
      queryClient.setQueryData(
        cacheKey,
        (previous: WorkflowRunLiveSnapshot | null | undefined) => previous === undefined
          || previous === null
          ? previous
          : { ...previous, cursor: event.cursor },
      );
      if (event.type === "hitl_required") {
        handlersRef.current.onHitlRequired?.(event.request);
        return;
      }
      if (event.type === "run_finished") {
        handlersRef.current.onRunFinished?.();
      }
    }, { afterCursor: snapshotRef.current?.cursor ?? null });
  }, [runtime, queryClient, runId, hasSnapshot]);

  return {
    ...query,
    artifacts: query.data?.artifacts ?? [],
    revealedId,
  };
}
