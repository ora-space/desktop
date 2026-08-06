import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { WorkflowSnapshot, WorkflowSummary, WorkflowVersion } from "@ora/contracts";
import { useContractsClient } from "../../contracts-client-context";

const workflowLibraryKey = ["workflow", "library"] as const;
const workflowDraftKey = (workflowId: string) => ["workflow", "draft", workflowId] as const;
const workflowVersionsKey = (workflowId: string) => ["workflow", "versions", workflowId] as const;

/**
 * Loads the persisted workflow library summaries shown by the settings manager.
 * The list stays lean (no graphs); drafts hydrate on selection via `useWorkflowDraft`.
 */
export function useWorkflowLibrary() {
  const client = useContractsClient();
  return useQuery({
    queryKey: workflowLibraryKey,
    queryFn: async () => (await client.workflow.list({})).workflows,
  });
}

/** Loads one workflow's record and draft snapshot (with its full graph envelope). */
export function useWorkflowDraft(workflowId: string | null | undefined) {
  const client = useContractsClient();
  return useQuery({
    queryKey: workflowDraftKey(workflowId ?? ""),
    queryFn: async () => client.workflow.get({ workflowId: workflowId! }),
    enabled: workflowId != null && workflowId !== "",
  });
}

/** Loads the published (non-draft) version summaries of one workflow. */
export function useWorkflowVersions(workflowId: string | null | undefined) {
  const client = useContractsClient();
  return useQuery({
    queryKey: workflowVersionsKey(workflowId ?? ""),
    queryFn: async () => (await client.workflow.listVersions({ workflowId: workflowId! })).versions,
    enabled: workflowId != null && workflowId !== "",
  });
}

/** Creates a new workflow with an optional initial graph. */
export function useCreateWorkflow() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: { name: string; graph?: string }) =>
      client.workflow.create({ name: input.name, graph: input.graph ?? null }),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: workflowLibraryKey });
    },
  });
}

/** Renames one workflow while preserving its identity. */
export function useRenameWorkflow() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: { workflowId: string; name: string }) => client.workflow.update(input),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: workflowLibraryKey });
    },
  });
}

/** Soft-deletes one workflow and cascades to its snapshots. */
export function useDeleteWorkflow() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (workflowId: string) => client.workflow.delete({ workflowId }),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: workflowLibraryKey });
    },
  });
}

/** Replaces one workflow's draft graph envelope in place. */
export function useUpdateWorkflowDraft() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: { workflowId: string; graph: string }) =>
      client.workflow.updateDraft(input),
    onSuccess: (_result, variables) => {
      void queryClient.invalidateQueries({ queryKey: workflowDraftKey(variables.workflowId) });
    },
  });
}

/** Publishes one workflow's draft as an immutable snapshot. */
export function usePublishWorkflow() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: { workflowId: string; version?: string | null }) =>
      client.workflow.publish({ workflowId: input.workflowId, version: input.version ?? null }),
    onSuccess: (_result, variables) => {
      void queryClient.invalidateQueries({ queryKey: workflowDraftKey(variables.workflowId) });
      void queryClient.invalidateQueries({ queryKey: workflowVersionsKey(variables.workflowId) });
      void queryClient.invalidateQueries({ queryKey: workflowLibraryKey });
    },
  });
}

/** Copies a historical snapshot's graph back into the draft. */
export function useRollbackWorkflow() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: { workflowId: string; snapshotId: string }) =>
      client.workflow.rollback(input),
    onSuccess: (_result, variables) => {
      void queryClient.invalidateQueries({ queryKey: workflowDraftKey(variables.workflowId) });
    },
  });
}

/** Soft-deletes a non-active published snapshot. */
export function useDeleteWorkflowSnapshot() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: { workflowId: string; version: string }) =>
      client.workflow.deleteSnapshot(input),
    onSuccess: (_result, variables) => {
      void queryClient.invalidateQueries({ queryKey: workflowVersionsKey(variables.workflowId) });
    },
  });
}

export type { WorkflowSnapshot, WorkflowSummary, WorkflowVersion };
