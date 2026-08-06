import { useEffect, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useWorkflowRuntime } from "../../features/workflow-run/use-workflow-runtime";
import type {
  GraphWorkflowRun,
  HitlRequest,
  WorkflowDefinitionInput,
  WorkflowNodeConversationItem,
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
    onSuccess: async (run) => {
      queryClient.setQueryData(queryKeys.workflowRun(run.id), run);
      // Resync the conversation projection from the live snapshot so a missed
      // stream upsert cannot leave the node session looking unchanged after HITL.
      const snapshot = await runtime.runs.getLiveSnapshot(run.id);
      if (snapshot === null) {
        return;
      }
      queryClient.setQueryData(
        queryKeys.workflowArtifacts(run.id),
        withConversationIndex(snapshot),
      );
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
  const [revealed, setRevealed] = useState<{
    runId: string;
    artifactId: string;
  } | null>(null);
  const handlersRef = useRef(handlers);
  useEffect(() => {
    handlersRef.current = handlers;
  }, [handlers]);

  const query = useQuery<LiveSnapshotIndexed | null>({
    queryKey: queryKeys.workflowArtifacts(runId ?? ""),
    queryFn: async () => {
      const snapshot = await runtime.runs.getLiveSnapshot(runId!);
      if (snapshot === null) {
        return null;
      }
      return withConversationIndex(snapshot);
    },
    enabled: runId != null && runId !== "",
    // Once loaded, the cursor stream owns freshness. Automatic refetch could
    // overwrite an event applied after the server produced its snapshot.
    staleTime: Number.POSITIVE_INFINITY,
  });
  const snapshotRef = useRef(query.data);
  useEffect(() => {
    snapshotRef.current = query.data;
  }, [query.data]);
  const hasSnapshot = query.data !== undefined && query.data !== null;

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
          (previous: LiveSnapshotIndexed | null | undefined) => {
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
        setRevealed({ runId, artifactId: artifact.id });
        return;
      }
      if (event.type === "node_conversation_item_upserted") {
        const item = structuredClone(event.item);
        queryClient.setQueryData(
          cacheKey,
          (previous: LiveSnapshotIndexed | null | undefined) => {
            if (previous === undefined || previous === null) {
              return previous;
            }
            let resolvedIndex = previous.conversationIndexById.get(item.id) ?? -1;
            if (resolvedIndex < 0) {
              // Defensive fallback for cache migrations or external cache writes:
              // if an item exists but was not indexed, repair instead of duplicating.
              resolvedIndex = previous.conversation.findIndex((current) => current.id === item.id);
            }
            const existing = resolvedIndex < 0 ? undefined : previous.conversation[resolvedIndex];
            const conversation = previous.conversation.slice();
            if (resolvedIndex < 0) {
              conversation.push(item);
            } else if (
              existing !== undefined
              && isSameConversationItem(existing, item)
            ) {
              return {
                ...previous,
                cursor: event.cursor,
              };
            } else {
              conversation[resolvedIndex] = item;
            }
            return {
              ...previous,
              conversation,
              conversationIndexById: upsertConversationIndexById(
                previous.conversationIndexById,
                item,
                resolvedIndex,
                conversation.length,
              ),
              conversationByNodeId: upsertConversationByNodeId(previous.conversationByNodeId, item),
              cursor: event.cursor,
            };
          },
        );
        return;
      }
      queryClient.setQueryData(
        cacheKey,
        (previous: LiveSnapshotIndexed | null | undefined) => previous === undefined
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
  const conversation = query.data?.conversation ?? [];
  const conversationByNodeId = query.data?.conversationByNodeId ?? new Map<string, WorkflowNodeConversationItem[]>();
  const revealedId = revealed !== null && revealed.runId === runId ? revealed.artifactId : null;

  return {
    ...query,
    artifacts: query.data?.artifacts ?? [],
    conversation,
    conversationByNodeId,
    revealedId,
  };
}

interface LiveSnapshotIndexed extends WorkflowRunLiveSnapshot {
  conversationIndexById: Map<string, number>;
  conversationByNodeId: Map<string, WorkflowNodeConversationItem[]>;
}

/** Adds a node-scoped conversation index to the live snapshot once at load time. */
function withConversationIndex(snapshot: WorkflowRunLiveSnapshot): LiveSnapshotIndexed {
  return {
    ...snapshot,
    conversationIndexById: buildConversationIndexById(snapshot.conversation),
    conversationByNodeId: buildConversationByNodeId(snapshot.conversation),
  };
}

/** Builds an id -> index lookup for ordered upsert operations. */
function buildConversationIndexById(
  conversation: WorkflowNodeConversationItem[],
): Map<string, number> {
  const indexById = new Map<string, number>();
  for (let index = 0; index < conversation.length; index += 1) {
    indexById.set(conversation[index].id, index);
  }
  return indexById;
}

/** Updates the id index map after one upsert in the ordered projection. */
function upsertConversationIndexById(
  previous: Map<string, number>,
  item: WorkflowNodeConversationItem,
  resolvedIndex: number,
  conversationLength: number,
): Map<string, number> {
  const existingIndex = previous.get(item.id);
  if (existingIndex !== undefined) {
    return previous;
  }
  const next = new Map(previous);
  next.set(item.id, resolvedIndex >= 0 ? resolvedIndex : conversationLength - 1);
  return next;
}

/** Builds a nodeId -> ordered conversation list index from a full projection list. */
function buildConversationByNodeId(
  conversation: WorkflowNodeConversationItem[],
): Map<string, WorkflowNodeConversationItem[]> {
  const grouped = new Map<string, WorkflowNodeConversationItem[]>();
  for (const item of conversation) {
    const bucket = grouped.get(item.nodeId);
    if (bucket === undefined) {
      grouped.set(item.nodeId, [item]);
    } else {
      bucket.push(item);
    }
  }
  return grouped;
}

/** Updates one node bucket immutably for an item upsert without rebuilding all groups. */
function upsertConversationByNodeId(
  previous: Map<string, WorkflowNodeConversationItem[]>,
  item: WorkflowNodeConversationItem,
): Map<string, WorkflowNodeConversationItem[]> {
  const bucket = previous.get(item.nodeId);
  if (bucket === undefined) {
    const next = new Map(previous);
    next.set(item.nodeId, [item]);
    return next;
  }
  const index = bucket.findIndex((current) => current.id === item.id);
  if (index < 0) {
    const next = new Map(previous);
    next.set(item.nodeId, [...bucket, item]);
    return next;
  }
  const current = bucket[index];
  if (isSameConversationItem(current, item)) {
    return previous;
  }
  const next = new Map(previous);
  const updated = bucket.slice();
  updated[index] = item;
  next.set(item.nodeId, updated);
  return next;
}

/** Compares session items by semantic fields so no-op upserts do not trigger churn. */
function isSameConversationItem(
  left: WorkflowNodeConversationItem,
  right: WorkflowNodeConversationItem,
): boolean {
  if (
    left.id !== right.id
    || left.kind !== right.kind
    || left.runId !== right.runId
    || left.nodeId !== right.nodeId
    || left.sessionId !== right.sessionId
    || left.createdAt !== right.createdAt
    || left.updatedAt !== right.updatedAt
    || left.status !== right.status
  ) {
    return false;
  }
  if (left.kind === "message" && right.kind === "message") {
    return left.role === right.role && left.markdown === right.markdown;
  }
  if (left.kind === "activity" && right.kind === "activity") {
    return left.activityKind === right.activityKind
      && left.summary === right.summary
      && left.detail === right.detail;
  }
  return false;
}
