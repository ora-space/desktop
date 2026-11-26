import { useState } from "react";
import { useTranslation } from "react-i18next";
import { localizeContractError } from "../../i18n/contract-error";
import { useWorkflowDebugAttempt } from "../../state/data/workflow-runs";
import { RunStatusBadge } from "./run-status-mark";

/** Read the immutable dispatch configuration only when the user opens this exact attempt. */
export function RunDebugAttempt({
  runId,
  nodeRunId,
}: {
  runId: string;
  nodeRunId: string;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  return (
    <details onToggle={(event) => setOpen(event.currentTarget.open)}>
      <summary className="cursor-pointer break-all">
        {t("workflowRun.debug.attempt", { id: nodeRunId })}
      </summary>
      {open && <AttemptFacts runId={runId} nodeRunId={nodeRunId} />}
    </details>
  );
}

function AttemptFacts({
  runId,
  nodeRunId,
}: {
  runId: string;
  nodeRunId: string;
}) {
  const { t } = useTranslation();
  const query = useWorkflowDebugAttempt(runId, nodeRunId);
  const attempt = query.data;
  return (
    <div className="space-y-1.5 py-2">
      {query.isPending && <p role="status">{t("workflowRun.loading")}</p>}
      {query.error && (
        <p role="alert">{localizeContractError(query.error, t)}</p>
      )}
      {attempt && (
        <>
          <RunStatusBadge status={attempt.status} quiet />
          {attempt.superseded && (
            <p>
              {t("workflowRun.debug.superseded")} {attempt.retiredByOperationId}
            </p>
          )}
          {attempt.revision !== null && (
            <p>
              {t("workflowRun.debug.attemptRevision", {
                revision: attempt.revision,
              })}
            </p>
          )}
          <p>{t("workflowRun.debug.recordedConfig")}</p>
          {attempt.configuration === null ? (
            <p>{t("workflowRun.debug.unrecorded")}</p>
          ) : (
            <pre className="whitespace-pre-wrap break-words">
              {JSON.stringify(attempt.configuration, null, 2)}
            </pre>
          )}
          {attempt.output !== null && (
            <p className="whitespace-pre-wrap">
              {t("workflowRun.inspector.output")} · {attempt.output}
            </p>
          )}
          {attempt.error && (
            <p className="whitespace-pre-wrap">{attempt.error}</p>
          )}
        </>
      )}
    </div>
  );
}
