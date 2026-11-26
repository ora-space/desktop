import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@ora/ui";
import type {
  WorkflowDebugOperation,
  WorkflowDebugHistoryEntry,
  WorkflowDebugPreview,
} from "@ora/contracts";
import { localizeContractError } from "../../i18n/contract-error";
import { useWorkflowDebugHistory } from "../../state/data/workflow-runs";
import { RunDebugAttempt } from "./run-debug-attempt";

function Operation({ operation }: { operation: WorkflowDebugOperation }) {
  const { t } = useTranslation();
  return (
    <p>
      {operation.state === "applied"
        ? t("workflowRun.debug.applied", { revision: operation.revision })
        : operation.state === "stopping"
          ? t("workflowRun.debug.stopping")
          : t("workflowRun.debug.stopFailed", {
              reason: operation.reason,
              ids: operation.unsettledNodeRunIds.join(", "),
            })}
    </p>
  );
}

function Impact({ preview: p }: { preview: WorkflowDebugPreview }) {
  const { t } = useTranslation();
  return (
    <div className="space-y-2">
      <p>
        {p.target.nodeId} · {p.target.scopeId} {p.target.containerNodeRunId}{" "}
        {p.target.nodeRunId}
      </p>
      {p.target.round != null && (
        <p>{t("workflowRun.loopRounds.round", { round: p.target.round })}</p>
      )}
      {p.target.index != null && (
        <p>{t("workflowRun.debug.item", { item: p.target.index + 1 })}</p>
      )}
      <dl>
        {(["keep", "invalidate", "stop", "schedule"] as const).map((group) => (
          <div key={group}>
            <dt className="font-medium">{t(`workflowRun.debug.${group}`)}</dt>
            <dd>{p[group].join(", ") || "—"}</dd>
          </div>
        ))}
      </dl>
      {p.predictedBranch && (
        <p>{t("workflowRun.debug.branch", { branch: p.predictedBranch })}</p>
      )}
      {p.memberConfigurations && (
        <details>
          <summary>{t("workflowRun.debug.members")}</summary>
          <pre>{JSON.stringify(p.memberConfigurations, null, 2)}</pre>
        </details>
      )}
      <p>{t("workflowRun.debug.files")}</p>
      {p.externalActionsMayRepeat && <p>{t("workflowRun.debug.repeat")}</p>}
    </div>
  );
}

function History({
  entries,
  runId,
}: {
  entries: WorkflowDebugHistoryEntry[];
  runId: string;
}) {
  const { t } = useTranslation();
  return (
    <>
      {entries.length === 0 ? (
        <p>{t("workflowRun.debug.empty")}</p>
      ) : (
        <ol>
          {entries.map((e) => (
            <li key={e.operation.operationId}>
              <details>
                <summary>
                  {e.preview.target.nodeId} · {e.operation.operationId}
                </summary>
                <Operation operation={e.operation} />
                <Impact preview={e.preview} />
                <pre>{JSON.stringify(e.patch, null, 2)}</pre>
                {e.preview.target.nodeRunId && (
                  <RunDebugAttempt
                    runId={runId}
                    nodeRunId={e.preview.target.nodeRunId}
                  />
                )}
                {e.preview.inputSourceNodeRunId && (
                  <div>
                    <p>{t("workflowRun.debug.inputSource")}</p>
                    <RunDebugAttempt
                      runId={runId}
                      nodeRunId={e.preview.inputSourceNodeRunId}
                    />
                  </div>
                )}
                {e.firstNodeRunId && (
                  <div>
                    <p>{t("workflowRun.debug.firstAttempt")}</p>
                    <RunDebugAttempt
                      runId={runId}
                      nodeRunId={e.firstNodeRunId}
                    />
                  </div>
                )}
              </details>
            </li>
          ))}
        </ol>
      )}
    </>
  );
}

export function RunDebugHistory({ runId }: { runId: string }) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  return (
    <details onToggle={(event) => setOpen(event.currentTarget.open)}>
      <summary className="cursor-pointer px-3 py-1">
        {t("workflowRun.debug.history")}
      </summary>
      {open && <PersistedHistory runId={runId} />}
    </details>
  );
}

function PersistedHistory({ runId }: { runId: string }) {
  const { t } = useTranslation();
  const query = useWorkflowDebugHistory(runId);
  return (
    <div className="max-h-[60vh] space-y-3 overflow-auto border-b p-3 text-sm">
      {query.data && <History entries={query.data} runId={runId} />}
      {query.isPending && <p role="status">{t("workflowRun.loading")}</p>}
      {query.error && (
        <p role="alert">{localizeContractError(query.error, t)}</p>
      )}
      <Button
        size="sm"
        disabled={query.isFetching}
        onClick={() => void query.refetch()}
      >
        {t("workflowRun.debug.refresh")}
      </Button>
    </div>
  );
}
