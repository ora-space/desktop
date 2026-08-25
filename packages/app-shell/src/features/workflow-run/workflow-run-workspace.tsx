import { useEffect, useMemo, useRef, useState } from "react";
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
import { useGraphWorkflowRunLive } from "../../state/hooks/use-graph-workflow-runs";
import {
  useCancelWorkflowRun,
  useRealWorkflowRun,
  useRestartWorkflowRun,
  useStartWorkflowRun,
} from "../../state/hooks/use-workflow-runs";
import {
  resolveCompletionAdvanceNodeId,
  resolveStageFocusNodeId,
  resolveTheaterFocus,
  shouldAdvanceAutomaticConversation,
  shouldReleaseLivePinToFollow,
  shouldStealFocusForArtifactReveal,
  type TheaterFocusStatusSample,
} from "./run-focus";
import { RunOverviewCanvas } from "./run-overview-canvas";
import { RunTheater } from "./run-theater";
import { RunStatusBadge } from "./run-status-mark";
import { isTerminalRunStatus, runStatusTone } from "./run-status-style";
import type { WorkflowRunViewMode } from "./run-view-mode";
import { LocationActionsButton } from "../workspace/location-actions-button";
import {
  WorkspaceReviewLayout,
  type WorkspaceReviewContext,
} from "../workspace/workspace-review-layout";

/** Below this width, Overview → Theater skips auto-opening the act inspector. */
const NARROW_THEATER_INSPECTOR_AUTO_OPEN_WIDTH = 1_000;

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
  const selectWorkflowRun = useWorkspaceSelectionStore(
    (s) => s.selectWorkflowRun,
  );
  const projectId = useWorkspaceSelectionStore((s) => s.selection.projectId);
  const runQuery = useRealWorkflowRun(runId);
  const run = runQuery.data?.run ?? null;
  const workspaceId = runQuery.data?.workspaceId ?? null;
  // WorkflowRun is Workspace-owned and no longer creates an implicit Task
  // projection, so its generic project review surface has no task diff count.
  const changedFileCount = 0;
  const startRun = useStartWorkflowRun();
  const cancelRun = useCancelWorkflowRun();
  const rerun = useRestartWorkflowRun();

  const [viewMode, setViewMode] = useState<WorkflowRunViewMode>("overview");
  const [focusNodeId, setFocusNodeId] = useState<string | null>(null);
  /** Node whose session dock is open — survives Overview ↔ Theater remounts. */
  const [conversationNodeId, setConversationNodeId] = useState<string | null>(
    null,
  );
  /** Node completion held until refreshed state exposes its next active node. */
  const [completionAdvanceFromNodeId, setCompletionAdvanceFromNodeId] =
    useState<string | null>(null);
  const [stopOpen, setStopOpen] = useState(false);
  /** One-shot: Overview node click should open Theater's act inspector. */
  const [openInspectorOnTheaterEnter, setOpenInspectorOnTheaterEnter] =
    useState(false);
  /** True while Changes/Files review panel is open (side or expanded). */
  const [reviewPanelOpen, setReviewPanelOpen] = useState(false);
  /** Re-fit Overview when the header control is activated (including while already there). */
  const [overviewFitRequestKey, setOverviewFitRequestKey] = useState(0);
  const stageAreaRef = useRef<HTMLDivElement | null>(null);

  /** Same-node status edge: live pin just finished -> resume auto-follow. */
  const focusStatusSampleRef = useRef<TheaterFocusStatusSample | null>(null);
  /** Open automatic-session edge: its running turn just finished -> open its successor. */
  const conversationStatusSampleRef = useRef<TheaterFocusStatusSample | null>(
    null,
  );
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
      // Drop live/auto pins in every view so Overview ↔ Theater cannot revive a
      // stale path focus after the run ends; explicit post-run picks still stick
      // because they happen after this clear.
      setFocusNodeId(null);
      setConversationNodeId(null);
      setCompletionAdvanceFromNodeId(null);
      if (viewModeRef.current === "overview") {
        toast.message(t("workflowRun.result.finishedToastTitle"), {
          description: t("workflowRun.result.finishedToastDescription"),
          action: {
            label: t("workflowRun.result.finishedToastAction"),
            onClick: () => {
              setOpenInspectorOnTheaterEnter(false);
              setViewMode("theater");
            },
          },
        });
      }
    },
  });

  // Reset local chrome when switching runs; mode is primed once below. State
  // resets follow the render-adjust pattern; the live-pin sample is ref-only.
  const [previousRunId, setPreviousRunId] = useState(runId);
  if (previousRunId !== runId) {
    setPreviousRunId(runId);
    setFocusNodeId(null);
    setConversationNodeId(null);
    setCompletionAdvanceFromNodeId(null);
    setStopOpen(false);
    setOpenInspectorOnTheaterEnter(false);
    setReviewPanelOpen(false);
  }
  useEffect(() => {
    focusStatusSampleRef.current = null;
    conversationStatusSampleRef.current = null;
  }, [runId]);

  // Interactive completion and automatic-session status edges share this intent. Consume it
  // exactly when refreshed state exposes the successor, without another effect-driven render.
  if (run !== null && completionAdvanceFromNodeId !== null) {
    const nextNodeId = resolveCompletionAdvanceNodeId(
      run,
      completionAdvanceFromNodeId,
    );
    if (nextNodeId !== null) {
      setCompletionAdvanceFromNodeId(null);
      setConversationNodeId(nextNodeId);
      setFocusNodeId(nextNodeId);
      setOpenInspectorOnTheaterEnter(false);
      setViewMode("theater");
    } else if (isTerminalRunStatus(run.status)) {
      setCompletionAdvanceFromNodeId(null);
      setConversationNodeId(null);
      setFocusNodeId(null);
    }
  }

  const conversationNodeIdRef = useRef<string | null>(null);
  conversationNodeIdRef.current = conversationNodeId;

  // An open automatic node has no completion button to create the existing advance intent.
  // Detect the same running -> terminal edge and feed it into that shared successor resolver.
  useEffect(() => {
    if (run === null || conversationNodeId === null) {
      conversationStatusSampleRef.current = null;
      return;
    }
    const currentStatus = run.nodeStates[conversationNodeId]?.status;
    const interactive =
      run.definitionSnapshot.nodes.find(
        (node) => node.id === conversationNodeId,
      )?.data.agentConfig?.interactive === true;
    if (
      shouldAdvanceAutomaticConversation(
        conversationStatusSampleRef.current,
        conversationNodeId,
        currentStatus,
        interactive,
      )
    ) {
      conversationStatusSampleRef.current = null;
      setCompletionAdvanceFromNodeId(conversationNodeId);
      return;
    }
    if (currentStatus !== undefined) {
      conversationStatusSampleRef.current = {
        nodeId: conversationNodeId,
        status: currentStatus,
      };
    }
  }, [run, conversationNodeId]);

  /**
   * Session dock is sticky attention: pin the act and ignore auto-follow /
   * artifact steals until the reader closes it or picks another path node.
   */
  function setSessionConversationNodeId(nodeId: string | null): void {
    setConversationNodeId(nodeId);
    if (nodeId !== null) {
      setFocusNodeId(nodeId);
    }
  }

  /** Effective Theater/Overview focus — session pin wins over any raced auto focus. */
  const stageFocusNodeId = resolveStageFocusNodeId(
    conversationNodeId,
    focusNodeId,
  );

  // If anything drifted focus while a session is open, snap it back.
  if (conversationNodeId !== null && focusNodeId !== conversationNodeId) {
    setFocusNodeId(conversationNodeId);
  }

  // Live pin release: only when the focused act itself just left live -> terminal.
  // History pins stay; an open node session is also sticky.
  useEffect(() => {
    if (run === null || focusNodeId === null) {
      focusStatusSampleRef.current = null;
      return;
    }
    const currentStatus = run.nodeStates[focusNodeId]?.status;
    if (
      shouldReleaseLivePinToFollow(
        conversationNodeIdRef.current,
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

  // New artifact on the stage: one-shot focus on the producing act.
  // Skip once terminal / while a node session is open; still consume the reveal
  // so closing the session does not suddenly jump to a stale artifact.
  const lastFocusedRevealRef = useRef<string | null>(null);
  useEffect(() => {
    lastFocusedRevealRef.current = null;
  }, [runId]);
  useEffect(() => {
    if (
      run === null ||
      isTerminalRunStatus(run.status) ||
      artifactsQuery.revealedId === null ||
      viewMode !== "theater" ||
      lastFocusedRevealRef.current === artifactsQuery.revealedId
    ) {
      return;
    }
    const artifact = artifactsQuery.artifacts.find(
      (item) => item.id === artifactsQuery.revealedId,
    );
    if (artifact === undefined) {
      return;
    }
    lastFocusedRevealRef.current = artifactsQuery.revealedId;
    const preferredFocus = resolveStageFocusNodeId(
      conversationNodeIdRef.current,
      focusNodeId,
    );
    const stagePrimary = resolveTheaterFocus(run, preferredFocus).primaryId;
    if (
      !shouldStealFocusForArtifactReveal({
        conversationNodeId: conversationNodeIdRef.current,
        stagePrimaryId: stagePrimary,
        artifactNodeId: artifact.nodeId,
      })
    ) {
      return;
    }
    setFocusNodeId(artifact.nodeId);
  }, [
    run,
    focusNodeId,
    artifactsQuery.revealedId,
    artifactsQuery.artifacts,
    viewMode,
  ]);

  // Prime view once per selected run: pending/terminal -> Overview, live -> Theater.
  // Later status ticks must not steal Overview if the user chose it mid-run.
  const [primedRunId, setPrimedRunId] = useState<string | null>(null);
  if (run !== null && run.id === runId && primedRunId !== runId) {
    setPrimedRunId(runId);
    if (run.status === "running" || run.status === "awaiting_input") {
      setViewMode("theater");
    } else {
      setViewMode("overview");
    }
  }

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
  const canStop =
    run !== null &&
    (run.status === "running" || run.status === "awaiting_input");
  const canRunAgain = run !== null && isTerminalRunStatus(run.status);
  const runTone = run !== null ? runStatusTone(run.status) : null;
  const actionBusy =
    startRun.isPending || cancelRun.isPending || rerun.isPending;

  // WorkflowRun has no implicit worktree Task. Its detail already carries the
  // project projection needed to scope the generic review surface.
  const reviewContext = useMemo<WorkspaceReviewContext>(() => {
    const runProjectId = runQuery.data?.projectId ?? projectId;
    return runProjectId !== null
      ? { kind: "project", projectId: runProjectId }
      : { kind: "none" };
  }, [projectId, runQuery.data?.projectId]);

  // If the run finishes while the stop dialog is open, dismiss it so Confirm
  // (which preventDefault + early-returns when !canStop) cannot leave a stuck modal.
  if (stopOpen && !canStop && !cancelRun.isPending) {
    setStopOpen(false);
  }

  function focusNode(nodeId: string): void {
    // Explicit path / carousel picks leave the session dock.
    if (conversationNodeId !== null && conversationNodeId !== nodeId) {
      setConversationNodeId(null);
    }
    setFocusNodeId(nodeId);
  }

  function focusNodeFromOverview(nodeId: string): void {
    setConversationNodeId(null);
    setFocusNodeId(nodeId);
    const waiting =
      run !== null && run.nodeStates[nodeId]?.status === "awaiting_input";
    const stageWidth =
      stageAreaRef.current?.getBoundingClientRect().width ??
      Number.POSITIVE_INFINITY;
    // Narrow stages cannot host the act card and inspector without crushing the card.
    const wideEnoughForInspector =
      stageWidth >= NARROW_THEATER_INSPECTOR_AUTO_OPEN_WIDTH;
    enterTheater({
      openInspector: !waiting && wideEnoughForInspector,
    });
  }

  function enterTheater(options?: { openInspector?: boolean }): void {
    setOpenInspectorOnTheaterEnter(options?.openInspector === true);
    setViewMode("theater");
  }

  /**
   * Header Theater: keep an explicit path pin across Overview ↔ Theater.
   * A second header click while already on Theater (terminal + pin) returns
   * to the result act.
   */
  function enterTheaterFromHeader(): void {
    if (
      viewMode === "theater" &&
      run !== null &&
      isTerminalRunStatus(run.status) &&
      focusNodeId !== null
    ) {
      setFocusNodeId(null);
      setConversationNodeId(null);
      return;
    }
    enterTheater();
  }

  function clearPathFocus(): void {
    setFocusNodeId(null);
    setConversationNodeId(null);
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
      // Restart re-runs the same run in place; the id is unchanged.
      await rerun.mutateAsync({ runId: run.id });
      selectWorkflowRun(run.id, run.projectId);
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
            {runTone ? (
              <RunStatusBadge
                status={run!.status}
                quiet
                className="hidden shrink-0 sm:inline-flex"
              />
            ) : (
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
                // Remount already fits when leaving Theater; bumping also
                // refits after pane resize while Overview stays mounted.
                setOverviewFitRequestKey((key) => key + 1);
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
              {startRun.isPending ? (
                <Spinner className="size-3.5" />
              ) : (
                <IconPlayerPlay className="size-3.5" />
              )}
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
              {rerun.isPending ? (
                <Spinner className="size-3.5" />
              ) : (
                <IconPlayerPlay className="size-3.5" />
              )}
              {t("workflowRun.runAgainAction")}
            </Button>
          )}
        </div>
        <LocationActionsButton workspaceId={workspaceId} />
        <WindowControls />
      </header>

      {runQuery.isLoading && run === null ? (
        <div className="flex flex-1 items-center justify-center gap-2 text-sm text-muted-foreground">
          <Spinner className="size-4" />
          {t("workflowRun.loading")}
        </div>
      ) : run === null ? (
        <div className="flex flex-1 items-center justify-center text-sm text-muted-foreground">
          {t("workflowRun.missing")}
        </div>
      ) : (
        <WorkspaceReviewLayout
          key={runId}
          context={reviewContext}
          onOpenChange={setReviewPanelOpen}
          preserveWorkspaceOnReviewOpen
        >
          <div
            ref={stageAreaRef}
            className="flex min-h-0 min-w-0 flex-1 flex-col"
          >
            {viewMode === "theater" ? (
              <RunTheater
                run={run}
                focusNodeId={stageFocusNodeId}
                onFocusNode={focusNode}
                onClearFocus={clearPathFocus}
                changedFileCount={changedFileCount}
                artifacts={artifactsQuery.artifacts}
                conversationByNodeId={artifactsQuery.conversationByNodeId}
                revealedArtifactId={artifactsQuery.revealedId}
                openInspectorOnMount={openInspectorOnTheaterEnter}
                onOpenInspectorOnMountConsumed={() => {
                  setOpenInspectorOnTheaterEnter(false);
                }}
                reviewPanelOpen={reviewPanelOpen}
                sessionConversationNodeId={conversationNodeId}
                onSessionConversationNodeIdChange={setSessionConversationNodeId}
                onNodeCompleted={setCompletionAdvanceFromNodeId}
                onShowOverview={() => {
                  setOpenInspectorOnTheaterEnter(false);
                  setViewMode("overview");
                }}
              />
            ) : (
              <RunOverviewCanvas
                run={run}
                focusedNodeId={stageFocusNodeId}
                onFocusNode={focusNodeFromOverview}
                artifacts={artifactsQuery.artifacts}
                fitRequestKey={overviewFitRequestKey}
              />
            )}
          </div>
        </WorkspaceReviewLayout>
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
