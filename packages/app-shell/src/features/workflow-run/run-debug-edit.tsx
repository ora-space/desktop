import { useMemo, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { Button } from "@ora/ui";
import {
  RemoteContractError,
  type GetWorkflowDebugContextRequest,
  type GetWorkflowDebugContextResponse,
  type WorkflowDebugOperation,
  type WorkflowDebugPreview,
} from "@ora/contracts";
import { parseWorkflowGraph } from "@ora/workflow-runtime";
import {
  createMockWorkflowCapabilities,
  deriveWorkflowVariableCatalog,
  normalizeWorkflowGlobalVariables,
} from "@ora/workflow-mock";
import { useContractsClient } from "../../contracts-client-context";
import { localizeContractError } from "../../i18n/contract-error";
import {
  invalidateWorkflowRun,
  useWorkflowDebugHistory,
} from "../../state/data/workflow-runs";
import { useWorkflowAgentModels } from "../../state/hooks/use-workflow-agent-models";
import {
  RunNodeConfigForm,
  type RunNodeConfig,
} from "../workflow-editor/run-node-config-form";
import { DebugImpact, DebugOperation } from "./run-debug-history";

/** Opens a fresh editing session only for the execution position selected by the inspector. */
export function RunDebugEdit({
  target,
}: {
  target: GetWorkflowDebugContextRequest;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  return open ? (
    <Editor target={target} onClose={() => setOpen(false)} />
  ) : (
    <Button size="sm" variant="outline" onClick={() => setOpen(true)}>
      {t("workflowRun.debug.edit")}
    </Button>
  );
}

function formData(context: GetWorkflowDebugContextResponse) {
  const { nodes, edges, globalVariables } = parseWorkflowGraph(context.graph);
  const node = nodes.find(
    (candidate) => candidate.id === context.target.nodeId,
  );
  if (!node) return;
  const data = node.data;
  let effective: RunNodeConfig;
  if (data.kind === "agent" && data.agentConfig)
    effective = { kind: data.kind, config: data.agentConfig };
  else if (data.kind === "condition" && data.cases)
    effective = { kind: data.kind, config: { cases: data.cases } };
  else if (data.kind === "loop" && data.loopConfig)
    effective = { kind: data.kind, config: data.loopConfig };
  else if (data.kind === "iteration" && data.iterationConfig)
    effective = { kind: data.kind, config: data.iterationConfig };
  else return;
  const globals = normalizeWorkflowGlobalVariables(globalVariables);
  const variables = deriveWorkflowVariableCatalog(
    nodes,
    edges,
    node.id,
    globals,
  );
  const members = nodes.filter(
    (candidate) =>
      candidate.parentId === node.id || candidate.data.containerId === node.id,
  );
  const memberIds = new Set(members.map((member) => member.id));
  const memberOutputs = deriveWorkflowVariableCatalog(
    nodes,
    [],
    undefined,
    globals,
  ).filter((entry) => memberIds.has(entry.sourceNodeId));
  if (effective.kind === "loop") {
    variables.push(...memberOutputs);
    for (const variable of effective.config.variables ?? [])
      variables.push({
        sourceNodeId: node.id,
        selector: [node.id, variable.name],
        variableName: variable.name,
        valueType: variable.valueType,
      });
  }
  return { effective, variables, memberOutputs };
}

function Editor({
  target,
  onClose,
}: {
  target: GetWorkflowDebugContextRequest;
  onClose: () => void;
}) {
  const { t, i18n } = useTranslation();
  const client = useContractsClient();
  const cache = useQueryClient();
  const models = useWorkflowAgentModels();
  const history = useWorkflowDebugHistory(target.runId);
  const [ticket, setTicket] = useState<{
    preview: WorkflowDebugPreview;
    requestId: string;
  }>();
  const [receipt, setReceipt] = useState<WorkflowDebugOperation>();
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);
  const lock = useRef(false);
  const context = useQuery({
    queryKey: ["workflowRun", "debugEditContext", target],
    queryFn: ({ signal }) =>
      client.workflowRun.debugContext(target, { signal }),
    gcTime: 0,
    refetchOnWindowFocus: false,
    retry: false,
  });
  const input = useMemo(
    () => context.data && formData(context.data),
    [context.data],
  );
  const operation =
    history.data?.find((e) => e.operation.requestId === ticket?.requestId)
      ?.operation ?? receipt;
  async function submit(
    edit: Parameters<typeof client.workflowRun.debugPreview>[0],
  ) {
    if (lock.current) return;
    lock.current = true;
    setBusy(true);
    setError(undefined);
    try {
      const preview = await client.workflowRun.debugPreview(edit);
      setTicket({ preview, requestId: crypto.randomUUID() });
    } catch (cause) {
      setError(localizeContractError(cause, t));
      if (
        cause instanceof RemoteContractError &&
        cause.code === "workflow_debug_stale_preview"
      )
        await context.refetch();
    } finally {
      lock.current = false;
      setBusy(false);
    }
  }
  async function confirm() {
    if (!ticket || lock.current) return;
    lock.current = true;
    setBusy(true);
    setError(undefined);
    try {
      setReceipt(
        await client.workflowRun.debugCommit({
          runId: target.runId,
          previewId: ticket.preview.previewId,
          requestId: ticket.requestId,
        }),
      );
    } catch (cause) {
      if (
        cause instanceof RemoteContractError &&
        cause.code.startsWith("workflow_debug_")
      ) {
        setTicket(undefined);
        setError(localizeContractError(cause, t));
        await context.refetch();
      } else {
        setError(t("workflowRun.debug.uncertain"));
      }
    } finally {
      invalidateWorkflowRun(cache, target.runId);
      lock.current = false;
      setBusy(false);
    }
  }
  return (
    <div className="space-y-2">
      {error && !operation && <p role="alert">{error}</p>}
      {operation ? (
        <DebugOperation operation={operation} />
      ) : ticket ? (
        <>
          <DebugImpact preview={ticket.preview} />
          <Button disabled={busy} onClick={() => void confirm()}>
            {t("workflowRun.debug.confirm")}
          </Button>
        </>
      ) : context.isPending ? (
        <p role="status">{t("workflowRun.loading")}</p>
      ) : context.error ? (
        <p role="alert">{localizeContractError(context.error, t)}</p>
      ) : input && context.data ? (
        <fieldset disabled={busy}>
          <legend>
            {context.data.target.nodeId} · {context.data.target.scopeId}{" "}
            {context.data.target.nodeRunId}
            {context.data.target.round != null &&
              " · " +
                t("workflowRun.loopRounds.round", {
                  round: context.data.target.round,
                })}
            {context.data.target.index != null &&
              " · " +
                t("workflowRun.debug.item", {
                  item: context.data.target.index + 1,
                })}
          </legend>
          <RunNodeConfigForm
            {...input}
            target={target}
            baseRevision={context.data.baseRevision}
            capabilities={createMockWorkflowCapabilities(
              i18n.resolvedLanguage === "en-US" ? "en-US" : "zh-CN",
              models.agentModels,
            )}
            onCancel={onClose}
            onSubmit={(edit) => void submit(edit)}
          />
        </fieldset>
      ) : (
        <p role="alert">{t("workflowRun.debug.unrecorded")}</p>
      )}
      {(ticket || context.error || !input) && (
        <Button variant="outline" disabled={busy} onClick={onClose}>
          {t("common.close")}
        </Button>
      )}
    </div>
  );
}
