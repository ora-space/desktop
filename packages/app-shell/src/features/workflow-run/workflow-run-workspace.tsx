import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  Button,
  Spinner,
  toast,
} from "@ora/ui";
import {
  IconLayoutSidebarLeftExpand,
  IconMap,
  IconPlayerPlay,
  IconPlayerStop,
  IconTheater,
} from "@tabler/icons-react";
import { DragRegion } from "../../components/drag-region";
import { WindowControls } from "../../components/window-controls";
import { useUiStore } from "../../state/stores/ui-store";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import {
  useCancelGraphWorkflowRun,
  useGraphWorkflowRun,
  useGraphWorkflowRunLive,
  useRerunGraphWorkflowRun,
  useStartGraphWorkflowRun,
} from "../../state/hooks/use-graph-workflow-runs";
import {
  shouldReleaseFocusToFollow,
  type TheaterFocusStatusSample,
} from "./run-focus";
import { RunOverviewCanvas } from "./run-overview-canvas";
import { RunTheater } from "./run-theater";
import { RunStatusBadge } from "./run-status-mark";
import { isTerminalRunStatus, runStatusTone } from "./run-status-style";
import type { WorkflowRunViewMode } from "./run-view-mode";

interface WorkflowRunWorkspaceProps {
  runId: string;
}

/**
 * Graph workflow run workspace: Theater / Overview.
 * Outcomes open in the Theater act inspector rail.
 */
export function WorkflowRunWorkspace({ runId }: WorkflowRunWorkspaceProps) {
  const { t } = useTranslation();
  const sidebarCollapsed = useUiStore((s) => s.sidebarCollapsed);
  const setSidebarCollapsed = useUiStore((s) => s.setSidebarCollapsed);
  const selectWorkflowRun = useWorkspaceSelectionStore((s) => s.selectWorkflowRun);
  const runQuery = useGraphWorkflowRun(runId);
  const run = runQuery.data ?? null;
  const startRun = useStartGraphWorkflowRun();
  const cancelRun = useCancelGraphWorkflowRun();
  const rerun = useRerunGraphWorkflowRun();

  const [viewMode, setViewMode] = useState<WorkflowRunViewMode>("overview");
  const [focusNodeId, setFocusNodeId] = useState<string | null>(null);
  const [stopOpen, setStopOpen] = useState(false);
  /** One-shot: Overview node click should open Theater's act inspector. */
  const [openInspectorOnTheaterEnter, setOpenInspectorOnTheaterEnter] = useState(
    false,
  );

  /** Same-node status edge: live pin just finished -> resume auto-follow. */
  const focusStatusSampleRef = useRef<TheaterFocusStatusSample | null>(null);
  const viewModeRef = useRef(viewMode);
  viewModeRef.current = viewMode;

  // Shared run subscribe: artifacts cache + HITL toast + result-act focus clear.
  const artifactsQuery = useGraphWorkflowRunLive(runId, {
    onHitlRequired: (request) => {
      const clarify = request.schema.kind === "clarify";
      toast.message(t("workflowRun.hitl.toastTitle"), {
        description: clarify
          ? t("workflowRun.hitl.toastClarifyDescription")
          : t("workflowRun.hitl.toastDescription"),
        action: {
          label: t("workflowRun.hitl.toastAction"),
          onClick: () => {
            setFocusNodeId(request.nodeId);
            setOpenInspectorOnTheaterEnter(false);
            setViewMode("theater");
          },
        },
      });
    },
    onRunFinished: () => {
      if (viewModeRef.current === "theater") {
        setFocusNodeId(null);
      }
    },
  });

  // Reset local chrome when switching runs; mode is primed once below.
  useEffect(() => {
    setFocusNodeId(null);
    setStopOpen(false);
    setOpenInspectorOnTheaterEnter(false);
    focusStatusSampleRef.current = null;
  }, [runId]);

  // Live pin release: only when the focused act itself just left live -> terminal.
  // History pins (clicked while already non-live) stay until the user picks again.
  useEffect(() => {
    if (run === null || focusNodeId === null) {
      focusStatusSampleRef.current = null;
      return;
    }
    const currentStatus = run.nodeStates[focusNodeId]?.status;
    if (
      shouldReleaseFocusToFollow(
        focusStatusSampleRef.current,
        focusNodeId,
        currentStatus,
      )
    ) {
      focusStatusSampleRef.current = null;
      setFocusNodeId(null);
      return;
    }
    if (currentStatus !== undefined) {
      focusStatusSampleRef.current = {
        nodeId: focusNodeId,
        status: currentStatus,
      };
    }
  }, [run, focusNodeId]);

  // New artifact on the stage: keep Theater focus on the producing act.
  // Skip once terminal so a stale reveal cannot hide the result act.
  useEffect(() => {
    if (
      run === null
      || isTerminalRunStatus(run.status)
      || artifactsQuery.revealedId === null
      || viewMode !== "theater"
    ) {
      return;
    }
    const artifact = artifactsQuery.artifacts.find(
      (item) => item.id === artifactsQuery.revealedId,
    );
    if (artifact !== undefined) {
      setFocusNodeId(artifact.nodeId);
    }
  }, [
    run,
    artifactsQuery.revealedId,
    artifactsQuery.artifacts,
    viewMode,
  ]);

  // Prime view once per selected run: pending/terminal -> Overview, live -> Theater.
  // Later status ticks must not steal Overview if the user chose it mid-run.
  const primedRunIdRef = useRef<string | null>(null);
  useEffect(() => {
    if (run === null || run.id !== runId) {
      return;
    }
    if (primedRunIdRef.current === runId) {
      return;
    }
    primedRunIdRef.current = runId;
    if (run.status === "running" || run.status === "awaiting_input") {
      setViewMode("theater");
    } else {
      setViewMode("overview");
    }
  }, [run, runId]);

  // Esc from Theater returns to Overview.
  useEffect(() => {
    if (viewMode !== "theater") {
      return;
    }
    function onKeyDown(event: KeyboardEvent): void {
      if (event.key !== "Escape" || event.defaultPrevented) {
        return;
      }
      setOpenInspectorOnTheaterEnter(false);
      setViewMode("overview");
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [viewMode]);

  const canStart = run?.status === "pending";
  const canStop = run !== null
    && (run.status === "running" || run.status === "awaiting_input");
  const canRunAgain = run !== null && isTerminalRunStatus(run.status);
  const runTone = run !== null ? runStatusTone(run.status) : null;
  const actionBusy = startRun.isPending || cancelRun.isPending || rerun.isPending;

  // If the run finishes while the stop dialog is open, dismiss it so Confirm
  // (which preventDefault + early-returns when !canStop) cannot leave a stuck modal.
  useEffect(() => {
    if (stopOpen && !canStop && !cancelRun.isPending) {
      setStopOpen(false);
    }
  }, [stopOpen, canStop, cancelRun.isPending]);

  function focusNode(nodeId: string): void {
    setFocusNodeId(nodeId);
  }

  function focusNodeFromOverview(nodeId: string): void {
    setFocusNodeId(nodeId);
    const waiting = run !== null && run.nodeStates[nodeId]?.status === "awaiting_input";
    enterTheater({
      openInspector: !waiting,
    });
  }

  function enterTheater(options?: { openInspector?: boolean }): void {
    setOpenInspectorOnTheaterEnter(options?.openInspector === true);
    setViewMode("theater");
  }

  /** Header Theater: terminal runs show the result act, not a leftover path pin. */
  function enterTheaterFromHeader(): void {
    if (run !== null && isTerminalRunStatus(run.status)) {
      setFocusNodeId(null);
    }
    enterTheater();
  }

  async function handleStart(): Promise<void> {
    if (run === null || !canStart) {
      return;
    }
    try {
      await startRun.mutateAsync({
        runId: run.id,
      });
      enterTheater();
    } catch {
      toast.error(t("workflowRun.startFailed"));
    }
  }

  async function handleRunAgain(): Promise<void> {
    if (run === null || !canRunAgain) {
      return;
    }
    try {
      const next = await rerun.mutateAsync(run);
      selectWorkflowRun(next.id, next.projectId);
    } catch {
      toast.error(t("workflowRun.rerunFailed"));
    }
  }

  async function handleConfirmStop(): Promise<void> {
    // Race: run may have reached a terminal status between open and confirm.
    if (run === null || !canStop) {
      setStopOpen(false);
      return;
    }
    try {
      await cancelRun.mutateAsync({
        runId: run.id,
      });
    } catch {
      toast.error(t("workflowRun.cancelFailed"));
    } finally {
      setStopOpen(false);
    }
  }

  return (
    <main
      id="main-content"
      className="flex min-h-0 min-w-0 flex-1 flex-col bg-background"
    >
      <header className="flex h-14 shrink-0 items-center gap-2 border-b border-border px-3">
        {sidebarCollapsed && (
          <Button
            variant="ghost"
            size="icon"
            onClick={() => setSidebarCollapsed(false)}
            aria-label={t("sidebar.expand")}
          >
            <IconLayoutSidebarLeftExpand />
          </Button>
        )}
        <DragRegion className="min-w-0 flex-1">
          <div className="flex min-w-0 items-center gap-2">
            <p className="min-w-0 truncate text-sm font-medium tracking-[-0.01em]">
              {run?.name ?? t("workflowRun.loading")}
            </p>
            {runTone
              ? (
                <RunStatusBadge
                  status={run!.status}
                  quiet
                  className="hidden shrink-0 sm:inline-flex"
                />
              )
              : (
                <p className="truncate text-[11px] text-muted-foreground">
                  {t("workflowRun.placeholderSubtitle")}
                </p>
              )}
          </div>
        </DragRegion>

        <div
          className="flex shrink-0 items-center gap-1.5"
          role="group"
          aria-label={t("workflowRun.viewMode.label")}
        >
          <div className="inline-flex rounded-lg border border-border p-0.5">
            <Button
              type="button"
              size="sm"
              variant={viewMode === "theater" ? "secondary" : "ghost"}
              className="h-7 gap-1.5 px-2.5 text-xs"
              aria-pressed={viewMode === "theater"}
              onClick={() => enterTheaterFromHeader()}
            >
              <IconTheater className="size-3.5" />
              {t("workflowRun.viewMode.theater")}
            </Button>
            <Button
              type="button"
              size="sm"
              variant={viewMode === "overview" ? "secondary" : "ghost"}
              className="h-7 gap-1.5 px-2.5 text-xs"
              aria-pressed={viewMode === "overview"}
              onClick={() => {
                setOpenInspectorOnTheaterEnter(false);
                setViewMode("overview");
              }}
            >
              <IconMap className="size-3.5" />
              {t("workflowRun.viewMode.overview")}
            </Button>
          </div>
          {canStart && run && (
            <Button
              type="button"
              size="sm"
              className="h-7 gap-1.5 px-2.5 text-xs"
              disabled={actionBusy}
              onClick={() => {
                void handleStart();
              }}
            >
              {startRun.isPending
                ? <Spinner className="size-3.5" />
                : <IconPlayerPlay className="size-3.5" />}
              {t("workflowRun.startAction")}
            </Button>
          )}
          {canStop && run && (
            <Button
              type="button"
              size="sm"
              variant="outline"
              className="h-7 gap-1.5 px-2.5 text-xs"
              disabled={actionBusy}
              onClick={() => setStopOpen(true)}
            >
              <IconPlayerStop className="size-3.5" />
              {t("workflowRun.stopAction")}
            </Button>
          )}
          {canRunAgain && run && (
            <Button
              type="button"
              size="sm"
              className="h-7 gap-1.5 px-2.5 text-xs"
              disabled={actionBusy}
              onClick={() => {
                void handleRunAgain();
              }}
            >
              {rerun.isPending
                ? <Spinner className="size-3.5" />
                : <IconPlayerPlay className="size-3.5" />}
              {t("workflowRun.runAgainAction")}
            </Button>
          )}
        </div>
        <WindowControls />
      </header>

      {runQuery.isLoading && run === null
        ? (
          <div className="flex flex-1 items-center justify-center gap-2 text-sm text-muted-foreground">
            <Spinner className="size-4" />
            {t("workflowRun.loading")}
          </div>
        )
        : run === null
        ? (
          <div className="flex flex-1 items-center justify-center text-sm text-muted-foreground">
            {t("workflowRun.missing")}
          </div>
        )
        : viewMode === "theater"
        ? (
          <RunTheater
            run={run}
            focusNodeId={focusNodeId}
            onFocusNode={focusNode}
            artifacts={artifactsQuery.artifacts}
            revealedArtifactId={artifactsQuery.revealedId}
            openInspectorOnMount={openInspectorOnTheaterEnter}
            onShowOverview={() => {
              setOpenInspectorOnTheaterEnter(false);
              setViewMode("overview");
            }}
          />
        )
        : (
          <RunOverviewCanvas
            run={run}
            focusedNodeId={focusNodeId}
            onFocusNode={focusNodeFromOverview}
            artifacts={artifactsQuery.artifacts}
          />
        )}

      <AlertDialog open={stopOpen} onOpenChange={setStopOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("workflowRun.stopTitle")}</AlertDialogTitle>
            <AlertDialogDescription>
              {t("workflowRun.stopDescription", {
                name: run?.name ?? "",
              })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={cancelRun.isPending}>
              {t("common.cancel")}
            </AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              disabled={cancelRun.isPending || !canStop}
              onClick={(event) => {
                event.preventDefault();
                void handleConfirmStop();
              }}
            >
              {cancelRun.isPending
                ? t("workflowRun.stopping")
                : t("workflowRun.stopConfirmAction")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </main>
  );
}
