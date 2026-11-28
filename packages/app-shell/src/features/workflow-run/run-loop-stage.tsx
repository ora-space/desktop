import { useTranslation } from "react-i18next";
import { RunInspectorToggle } from "./run-inspector-toggle";
import type {
  GraphWorkflowRound,
  GraphWorkflowRun,
} from "@ora/workflow-runtime";
import { RunStatusBadge } from "./run-status-mark";
import { formatWorkflowNodeOutput } from "./format-node-output";
import { loopRoundOutcome } from "./loop-round-state";
import { projectRunPathStructure } from "./run-path-structure";

/** Shows committed outputs for one selected scope; live session text stays in the chat dock. */
export function RunLoopStage({
  run,
  loopId,
  round,
  showResult,
  onFocusNode,
  inspectorOpen,
  onToggleInspector,
}: {
  run: GraphWorkflowRun;
  loopId: string;
  round?: GraphWorkflowRound;
  showResult: boolean;
  onFocusNode: (nodeId: string) => void;
  inspectorOpen: boolean;
  onToggleInspector: () => void;
}) {
  const { t } = useTranslation();
  const loop = run.definitionSnapshot.nodes.find((node) => node.id === loopId);
  const parent = run.nodeStates[loopId];
  const stage = projectRunPathStructure(run.definitionSnapshot).find(
    (stage) => stage.nodeId === loopId,
  );
  const members =
    stage?.type === "region"
      ? stage.phases.flatMap((phase) => phase.nodeIds)
      : [];
  const reason =
    parent?.stopReason === "loop_max_iterations"
      ? "workflowRun.loopView.maxIterations"
      : parent?.stopReason === "loop_exit"
        ? "workflowRun.loopView.exit"
        : parent?.stopReason === "loop_succeeded"
          ? "workflowRun.loopView.condition"
          : parent?.status === "failed"
            ? "workflowRun.loopView.failed"
            : parent?.status === "cancelled"
              ? "workflowRun.loopView.cancelled"
              : "workflowRun.loopView.resultPending";
  return (
    <section
      className="rounded-2xl border border-border bg-card p-5 shadow-sm"
      aria-label={t(
        showResult
          ? "workflowRun.loopView.result"
          : "workflowRun.loopView.overview",
      )}
    >
      <div className="mb-4 flex flex-wrap items-center justify-between gap-2">
        <h2 className="mr-auto text-base font-semibold">
          {loop?.data.title} ·{" "}
          {showResult
            ? t("workflowRun.loopView.result")
            : round
              ? t("workflowRun.loopRounds.round", { round: round.roundIndex })
              : t("workflowRun.loopView.overview")}
        </h2>
        <RunStatusBadge
          status={
            showResult ? (parent?.status ?? "idle") : (round?.status ?? "idle")
          }
          quiet
        />
        <RunInspectorToggle open={inspectorOpen} onToggle={onToggleInspector} />
      </div>
      {showResult ? (
        <>
          <p className="mb-3 text-sm text-muted-foreground">{t(reason)}</p>
          {parent?.errorMessage && (
            <p className="text-sm text-destructive">{parent.errorMessage}</p>
          )}
          {parent?.status === "succeeded" &&
            parent.output?.summary !== undefined && (
              <pre className="max-h-96 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-muted/30 p-3 text-xs">
                {formatWorkflowNodeOutput(parent.output.summary)}
              </pre>
            )}
        </>
      ) : round === undefined ? (
        <p className="text-sm text-muted-foreground">
          {t("workflowRun.loopView.empty")}
        </p>
      ) : (
        <>
          <p className="mb-4 text-sm text-muted-foreground">
            {t(loopRoundOutcome(run, round))}
          </p>
          <div className="space-y-3">
            {members.map((id) => {
              const node = run.definitionSnapshot.nodes.find(
                (node) => node.id === id,
              )!;
              const state = round.nodeStates[id];
              return (
                <article key={id} className="border-t border-border pt-3">
                  <button
                    type="button"
                    className="flex w-full items-center justify-between gap-3 text-left text-sm font-medium"
                    onClick={() => onFocusNode(id)}
                  >
                    <span>{node.data.title}</span>
                    {state ? (
                      <RunStatusBadge status={state.status} quiet />
                    ) : (
                      <span className="text-xs text-muted-foreground">
                        {t("workflowRun.theater.notRunThisRound")}
                      </span>
                    )}
                  </button>
                  {state?.status === "succeeded" &&
                    state.output?.summary !== undefined && (
                      <pre className="mt-2 max-h-48 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-muted/30 p-3 text-xs">
                        {formatWorkflowNodeOutput(state.output.summary)}
                      </pre>
                    )}
                  {state?.errorMessage && (
                    <p className="mt-2 text-sm text-destructive">
                      {state.errorMessage}
                    </p>
                  )}
                  {node.data.kind === "agent" &&
                    state?.status === "running" && (
                      <p className="mt-2 text-xs text-muted-foreground">
                        {t("workflowRun.loopView.outputPending")}
                      </p>
                    )}
                </article>
              );
            })}
          </div>
        </>
      )}
    </section>
  );
}
