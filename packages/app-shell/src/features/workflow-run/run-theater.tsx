import { useEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import { useTranslation } from "react-i18next";
import { Badge, cn, toast } from "@ora/ui";
import { useUpdateWorkflowRunInput } from "../../state/hooks/use-workflow-runs";
import { filterArtifacts, latestArtifact } from "./artifact-filter";
import { RunActInspector } from "./run-act-inspector";
import { RunResultAct } from "./run-result-act";
import { RunTheaterActCard } from "./run-theater-act-card";
import { RunTheaterParallelStage } from "./run-theater-parallel-stage";
import { RunTheaterPathRail } from "./run-theater-path-rail";
import { resolveTheaterFocus } from "./run-focus";
import { isNodeWorking } from "./run-status-style";
import { isTerminalRunStatus } from "./run-status-style";
import {
  animateOverlayWidth,
  cancelOverlayWidthAnimation,
} from "./theater-overlay-motion";
import { useTheaterHitl } from "./use-theater-hitl";
import type {
  GraphWorkflowRun,
  WorkflowArtifact,
  WorkflowNodeConversationItem,
} from "@ora/workflow-runtime";
import "./theater-motion.css";

const DEFAULT_INSPECTOR_WIDTH = 320;
const MIN_INSPECTOR_WIDTH = 240;
const MAX_INSPECTOR_WIDTH = 480;
const INSPECTOR_COLLAPSE_THRESHOLD = 180;
const INSPECTOR_FADE_START = 120;
const PANEL_SETTLE_DURATION = 180;

interface RunTheaterProps {
  run: GraphWorkflowRun;
  focusNodeId: string | null;
  onFocusNode: (nodeId: string) => void;
  /** Clears path pin so the terminal result act can own the stage. */
  onClearFocus?: () => void;
  /** Total files the run-task worktree changed, shown on the terminal result act. */
  changedFileCount?: number;
  artifacts: WorkflowArtifact[];
  conversationByNodeId: Map<string, WorkflowNodeConversationItem[]>;
  revealedArtifactId: string | null;
  /** Opens the companion rail once when seeded (Overview → Theater enter). */
  openInspectorOnMount?: boolean;
  /** Clears the one-shot mount flag after the rail has been requested (or skipped). */
  onOpenInspectorOnMountConsumed?: () => void;
  /**
   * When Changes/Files is open, suppress *automatic* inspector opens (seeded
   * mount, artifact reveal). Intentional act clicks still open the rail so
   * Diff and the inspector can coexist on the stage.
   */
  reviewPanelOpen?: boolean;
  /** Result act CTA — return to Overview path map. */
  onShowOverview: () => void;
  /** Which node's session dock is open — lifted across Overview remounts. */
  sessionConversationNodeId?: string | null;
  onSessionConversationNodeIdChange?: (nodeId: string | null) => void;
}

/**
 * Focused act stage + path rail + overlay companion inspector.
 * HITL lives in `useTheaterHitl`; path chrome in `RunTheaterPathRail`.
 * Terminal + no path pin → result act. Esc (workspace) returns to Overview.
 */
export function RunTheater({
  run,
  focusNodeId,
  onFocusNode,
  onClearFocus,
  changedFileCount = 0,
  artifacts,
  conversationByNodeId,
  revealedArtifactId,
  openInspectorOnMount = false,
  onOpenInspectorOnMountConsumed,
  reviewPanelOpen = false,
  onShowOverview,
  sessionConversationNodeId = null,
  onSessionConversationNodeIdChange,
}: RunTheaterProps) {
  const { t } = useTranslation();
  const updateInput = useUpdateWorkflowRunInput();
  // Local draft of the start-node instruction while the user edits it. Committed to the run's
  // kickoff input only by the explicit save action, so per-keystroke refetches cannot clobber
  // an in-progress edit or fire one mutation per character.
  const [instructionDraft, setInstructionDraft] = useState<string | null>(null);
  const inspectorAnimationRef = useRef<number | null>(null);
  const inspectorWidthRef = useRef(DEFAULT_INSPECTOR_WIDTH);
  const inspectorCurrentWidthRef = useRef(0);
  const resizeDragRef = useRef<{ startX: number; startWidth: number } | null>(null);
  const [inspectorCollapsed, setInspectorCollapsed] = useState(true);
  const [inspectorVisualWidth, setInspectorVisualWidth] = useState(0);
  const pathScrollOpenSigRef = useRef<string>("");
  const pathRailRef = useRef<HTMLDivElement | null>(null);

  const focus = useMemo(
    () => resolveTheaterFocus(run, focusNodeId),
    [run, focusNodeId],
  );
  const primaryId = focus.primaryId;
  const parallel = focus.activeIds.length > 1;
  const parallelCarouselFocus = primaryId !== null
    && parallel
    && focus.activeIds.length > 1
    && focus.activeIds.includes(primaryId);
  const showParallelCarousel = parallelCarouselFocus;
  const showResultAct = isTerminalRunStatus(run.status) && focusNodeId === null;

  const {
    openHitls,
    primaryHasHitl,
    hitlExpanded,
    hitlComposer,
    renderHitlComposer,
    expandHitlForRequest,
    collapseHitl,
  } = useTheaterHitl({
    run,
    focusNodeId,
    primaryId,
    onFocusNode,
  });

  // Scroll path rail when the open-gate set changes — not on every primary tick.
  useEffect(() => {
    const requestSig = openHitls.map((item) => item.id).sort().join("|");
    const openSig = requestSig === "" ? "" : `${run.id}:${requestSig}`;
    if (openSig === "" || openSig === pathScrollOpenSigRef.current) {
      return;
    }
    pathScrollOpenSigRef.current = openSig;
    const targetId = openHitls.some((item) => item.nodeId === primaryId)
      ? primaryId
      : (openHitls[0]?.nodeId ?? primaryId);
    if (targetId === null || pathRailRef.current === null) {
      return;
    }
    const chip = pathRailRef.current.querySelector(
      `[data-path-node="${CSS.escape(targetId)}"]`,
    );
    if (chip instanceof HTMLElement) {
      chip.scrollIntoView({
        behavior: "smooth",
        inline: "nearest",
        block: "nearest",
      });
    }
  }, [openHitls, primaryId, run.id]);

  const nodeById = useMemo(
    () => new Map(run.definitionSnapshot.nodes.map((node) => [node.id, node])),
    [run.definitionSnapshot.nodes],
  );
  const primaryNode = primaryId === null ? undefined : nodeById.get(primaryId);
  const primaryState = primaryId !== null
    ? run.nodeStates[primaryId]
    : undefined;
  // The start instruction is editable whenever the run is not executing — a not-started pending
  // run or any terminal run — so the kickoff input can be changed before a restart re-runs it.
  const isEditableStart = (run.status === "pending" || isTerminalRunStatus(run.status))
    && primaryNode?.data?.kind === "start";

  // Drop an uncommitted draft the moment the run leaves pending (or the run switches) so a stale
  // draft cannot reappear on the start node after a restart. Implemented as a render-time reset
  // keyed on the last pending lifecycle state instead of an effect to avoid a cascading render.
  const [draftPendingKey, setDraftPendingKey] = useState("");
  const nextDraftPendingKey = run.status === "pending" ? run.id : "";
  if (nextDraftPendingKey !== draftPendingKey) {
    setDraftPendingKey(nextDraftPendingKey);
    setInstructionDraft(null);
  }
  const primaryArtifacts = useMemo(
    () =>
      primaryId === null
        ? []
        : filterArtifacts(artifacts, { type: "node", nodeId: primaryId }),
    [artifacts, primaryId],
  );
  const primaryRealConversation = primaryState?.conversation;
  const primaryConversation = useMemo(
    () => {
      // The real adapter projects the node's conversation from its run output; the mock
      // runtime provides it through the live snapshot instead.
      const mockItems = primaryId === null
        ? []
        : (conversationByNodeId.get(primaryId) ?? []);
      return primaryRealConversation != null && primaryRealConversation.length > 0
        ? primaryRealConversation
        : mockItems;
    },
    [primaryId, conversationByNodeId, primaryRealConversation],
  );
  const artifactCountByNode = useMemo(() => {
    const counts: Record<string, number> = {};
    for (const artifact of artifacts) {
      counts[artifact.nodeId] = (counts[artifact.nodeId] ?? 0) + 1;
    }
    return counts;
  }, [artifacts]);
  const parallelActs = useMemo(() => {
    if (!parallel) {
      return [];
    }
    return focus.activeIds.flatMap((nodeId) => {
      const node = nodeById.get(nodeId);
      const state = run.nodeStates[nodeId];
      if (node === undefined || state === undefined) {
        return [];
      }
      return [{
        nodeId,
        data: node.data,
        state,
        artifactCount: artifactCountByNode[nodeId] ?? 0,
        conversation: conversationByNodeId.get(nodeId) ?? [],
      }];
    });
  }, [
    parallel,
    focus.activeIds,
    nodeById,
    run.nodeStates,
    artifactCountByNode,
    conversationByNodeId,
  ]);

  const progress = useMemo(() => {
    const states = Object.values(run.nodeStates);
    const total = Math.max(states.length, 1);
    const done = states.filter(
      (state) =>
        state.status === "succeeded"
        || state.status === "failed"
        || state.status === "cancelled",
    ).length;
    return { done, total, percent: Math.round((done / total) * 100) };
  }, [run.nodeStates]);

  useEffect(() => {
    return () => cancelOverlayWidthAnimation(inspectorAnimationRef);
  }, []);

  function applyInspectorWidth(width: number): void {
    const next = Math.max(0, Math.min(MAX_INSPECTOR_WIDTH, width));
    inspectorCurrentWidthRef.current = next;
    setInspectorVisualWidth(next);
    setInspectorCollapsed(next < 1);
    if (next >= MIN_INSPECTOR_WIDTH) {
      inspectorWidthRef.current = next;
    }
  }

  /**
   * Opens the act inspector. Automatic triggers stay quiet while Diff/Files is
   * open; user clicks always open so the rail can sit beside an open Diff.
   */
  function openInspector(reason: "automatic" | "user"): void {
    if (reason === "automatic" && reviewPanelOpen) {
      return;
    }
    if (hitlExpanded) {
      collapseHitl();
    }
    setInspectorCollapsed(false);
    animateOverlayWidth({
      animationRef: inspectorAnimationRef,
      duration: PANEL_SETTLE_DURATION,
      fromWidth: inspectorCurrentWidthRef.current,
      onCollapsed: () => setInspectorCollapsed(true),
      onFrame: applyInspectorWidth,
      targetWidth: inspectorWidthRef.current,
    });
  }

  function closeInspector(): void {
    animateOverlayWidth({
      animationRef: inspectorAnimationRef,
      duration: PANEL_SETTLE_DURATION,
      fromWidth: inspectorCurrentWidthRef.current,
      onCollapsed: () => setInspectorCollapsed(true),
      onFrame: applyInspectorWidth,
      targetWidth: 0,
    });
  }

  // Commits the drafted start-node instruction to the run's kickoff input in one request and
  // clears the draft on success (the refetched run then surfaces the saved input).
  function saveInstructionDraft(): void {
    if (instructionDraft === null) {
      return;
    }
    updateInput.mutate({
      runId: run.id,
      input: instructionDraft,
    }, {
      onSuccess: () => setInstructionDraft(null),
      onError: () => toast.error(t("workflowRun.updateFailed")),
    });
  }

  // Expanded HITL and the inspector rail compete for the same stage edge.
  useEffect(() => {
    if (!hitlExpanded || inspectorCollapsed) {
      return;
    }
    closeInspector();
    // eslint-disable-next-line react-hooks/exhaustive-deps -- intentional
  }, [hitlExpanded]);

  // Seeded open from Overview → Theater. Skipped (and retried) while Diff/Files
  // owns the workspace so opening Changes does not auto-pop the rail.
  useEffect(() => {
    if (!openInspectorOnMount || reviewPanelOpen) {
      return;
    }
    const frame = window.requestAnimationFrame(() => {
      openInspector("automatic");
      onOpenInspectorOnMountConsumed?.();
    });
    return () => window.cancelAnimationFrame(frame);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- intentional
  }, [openInspectorOnMount, reviewPanelOpen]);

  function settleInspectorAfterUserResize(): void {
    const width = inspectorCurrentWidthRef.current;
    if (width <= 0 || width >= MIN_INSPECTOR_WIDTH) {
      return;
    }
    animateOverlayWidth({
      animationRef: inspectorAnimationRef,
      duration: PANEL_SETTLE_DURATION,
      fromWidth: width,
      onCollapsed: () => setInspectorCollapsed(true),
      onFrame: applyInspectorWidth,
      targetWidth: width < INSPECTOR_COLLAPSE_THRESHOLD
        ? 0
        : MIN_INSPECTOR_WIDTH,
    });
  }

  function onResizePointerDown(event: ReactPointerEvent<HTMLDivElement>): void {
    if (event.button !== 0) {
      return;
    }
    cancelOverlayWidthAnimation(inspectorAnimationRef);
    resizeDragRef.current = {
      startX: event.clientX,
      startWidth: inspectorCurrentWidthRef.current,
    };
    event.currentTarget.setPointerCapture(event.pointerId);
  }

  function onResizePointerMove(event: ReactPointerEvent<HTMLDivElement>): void {
    const drag = resizeDragRef.current;
    if (drag === null) {
      return;
    }
    applyInspectorWidth(drag.startWidth + (drag.startX - event.clientX));
  }

  function onResizePointerUp(event: ReactPointerEvent<HTMLDivElement>): void {
    if (resizeDragRef.current === null) {
      return;
    }
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    resizeDragRef.current = null;
    settleInspectorAfterUserResize();
  }

  // Artifact reveal: open the rail only when the stage just moved onto the
  // producing act. If we were already there, skip — re-tweening the inspector
  // collapses HITL and flashes the card for no navigation benefit.
  const previousPrimaryForRevealRef = useRef<string | null>(primaryId);
  useEffect(() => {
    const previousPrimary = previousPrimaryForRevealRef.current;
    previousPrimaryForRevealRef.current = primaryId;

    if (
      revealedArtifactId === null
      || showResultAct
      || sessionConversationNodeId !== null
    ) {
      return;
    }
    const artifact = artifacts.find((item) => item.id === revealedArtifactId);
    if (artifact === undefined || artifact.nodeId !== primaryId) {
      return;
    }
    if (previousPrimary === artifact.nodeId) {
      return;
    }
    openInspector("automatic");
    // eslint-disable-next-line react-hooks/exhaustive-deps -- intentional
  }, [
    revealedArtifactId,
    primaryId,
    showResultAct,
    sessionConversationNodeId,
    artifacts,
  ]);

  useEffect(() => {
    if (!showResultAct || inspectorCollapsed) {
      return;
    }
    closeInspector();
    // eslint-disable-next-line react-hooks/exhaustive-deps -- intentional
  }, [showResultAct]);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <RunTheaterPathRail
        run={run}
        primaryId={primaryId}
        activeIds={focus.activeIds}
        openHitls={openHitls}
        artifactCountByNode={artifactCountByNode}
        showResultAct={showResultAct}
        progress={progress}
        pathRailRef={pathRailRef}
        onFocusNode={onFocusNode}
        onExpandHitl={expandHitlForRequest}
        onShowResultAct={isTerminalRunStatus(run.status) ? onClearFocus : undefined}
      />

      <div className="relative flex min-h-0 flex-1 flex-col overflow-hidden">
        <div className="relative min-h-0 flex-1 overflow-hidden">
          <div className="absolute inset-0 flex flex-col overflow-auto p-6">
            <div
              className="pointer-events-none absolute inset-0 bg-[radial-gradient(ellipse_at_50%_30%,color-mix(in_oklch,var(--muted)_55%,transparent),transparent_65%)]"
              aria-hidden
            />
            <div className="relative mx-auto my-auto w-full max-w-xl shrink-0">
              {showResultAct
                ? (
                  <RunResultAct
                    run={run}
                    artifactCount={artifacts.length}
                    changedFileCount={changedFileCount}
                    onShowOverview={onShowOverview}
                    onOpenArtifacts={artifacts.length > 0
                      ? () => {
                        const recent = latestArtifact(artifacts);
                        if (recent !== null) {
                          onFocusNode(recent.nodeId);
                        }
                        openInspector("user");
                      }
                      : undefined}
                  />
                )
                : showParallelCarousel
                ? (
                  <div className="space-y-3">
                    <RunTheaterParallelStage
                      acts={parallelActs}
                      primaryId={primaryId!}
                      onFocusNode={onFocusNode}
                      onOpenInspector={() => openInspector("user")}
                      sessionConversationNodeId={sessionConversationNodeId}
                      onSessionConversationNodeIdChange={onSessionConversationNodeIdChange}
                      primaryInteraction={primaryHasHitl
                        ? ({ accessory }) => renderHitlComposer(accessory ?? undefined)
                        : undefined}
                    />
                    {!primaryHasHitl && hitlComposer !== null && (
                      <div className="px-0.5">
                        {hitlComposer}
                      </div>
                    )}
                  </div>
                )
                : primaryNode && primaryState
                ? (
                  <div
                    key={primaryNode.id}
                    className="animate-in fade-in zoom-in-95 slide-in-from-bottom-2 duration-300 ease-[cubic-bezier(0.22,1,0.36,1)] fill-mode-both motion-reduce:animate-none"
                  >
                    <RunTheaterActCard
                      data={primaryNode.data}
                      state={primaryState}
                      live={isNodeWorking(primaryState.status)}
                      artifactCount={primaryArtifacts.length}
                      conversation={primaryConversation}
                      conversationOpen={sessionConversationNodeId === primaryNode.id}
                      onConversationOpenChange={(open) => {
                        onSessionConversationNodeIdChange?.(
                          open ? primaryNode.id : null,
                        );
                      }}
                      variant="stage"
                      onSelect={() => openInspector("user")}
                      interaction={primaryHasHitl
                        ? ({ accessory }) => renderHitlComposer(accessory ?? undefined)
                        : undefined}
                    />
                    {!primaryHasHitl && hitlComposer !== null && (
                      <div className="mt-3">
                        {hitlComposer}
                      </div>
                    )}
                  </div>
                )
                : (
                  <p className="text-center text-sm text-muted-foreground">
                    {t("workflowRun.theater.empty")}
                  </p>
                )}

              {!showResultAct && (
                <div className="mt-6 flex flex-wrap items-center justify-center gap-2">
                  {parallel && (
                    <Badge variant="secondary" className="tabular-nums">
                      {t("workflowRun.theater.parallelCount", {
                        count: focus.activeIds.length,
                      })}
                    </Badge>
                  )}
                </div>
              )}
              {!showResultAct && (
                <p className="mt-3 text-center text-[10px] text-muted-foreground/70">
                  {hitlExpanded
                    ? t("workflowRun.theater.hitlHint")
                    : inspectorCollapsed
                    ? t("workflowRun.theater.inspectorHint")
                    : t("workflowRun.theater.returnOverviewHint")}
                </p>
              )}
            </div>
          </div>

          <aside
            className={cn(
              "absolute inset-y-0 right-0 z-30 flex",
              inspectorVisualWidth < 1 && "pointer-events-none",
            )}
            style={{ width: inspectorVisualWidth }}
            aria-hidden={inspectorCollapsed}
          >
            <div
              role="separator"
              aria-orientation="vertical"
              aria-label={t("workflowRun.inspector.resize")}
              title={t("workflowRun.inspector.resize")}
              tabIndex={inspectorCollapsed ? -1 : 0}
              className={cn(
                "relative z-20 flex w-px shrink-0 cursor-col-resize items-center justify-center bg-transparent transition-colors",
                "after:absolute after:inset-y-0 after:left-1/2 after:w-3 after:-translate-x-1/2",
                "hover:bg-ring/60 focus-visible:bg-ring focus-visible:outline-none",
                "hover:[&>span]:opacity-100 focus-visible:[&>span]:opacity-100",
                inspectorVisualWidth < 1 && "opacity-0",
              )}
              onPointerDown={onResizePointerDown}
              onPointerMove={onResizePointerMove}
              onPointerUp={onResizePointerUp}
              onPointerCancel={onResizePointerUp}
              onDoubleClick={() => {
                cancelOverlayWidthAnimation(inspectorAnimationRef);
                inspectorWidthRef.current = DEFAULT_INSPECTOR_WIDTH;
                animateOverlayWidth({
                  animationRef: inspectorAnimationRef,
                  duration: PANEL_SETTLE_DURATION,
                  fromWidth: inspectorCurrentWidthRef.current,
                  onCollapsed: () => setInspectorCollapsed(true),
                  onFrame: applyInspectorWidth,
                  targetWidth: DEFAULT_INSPECTOR_WIDTH,
                });
              }}
              onKeyDown={(event) => {
                if (event.key === "ArrowLeft") {
                  event.preventDefault();
                  applyInspectorWidth(inspectorCurrentWidthRef.current + 16);
                }
                if (event.key === "ArrowRight") {
                  event.preventDefault();
                  applyInspectorWidth(inspectorCurrentWidthRef.current - 16);
                  settleInspectorAfterUserResize();
                }
              }}
            >
              <span
                className="pointer-events-none z-10 h-5 w-0.5 rounded-full bg-muted-foreground/35 opacity-0 transition-opacity"
                aria-hidden
              />
            </div>
            <div
              className="flex min-h-0 min-w-0 flex-1 overflow-hidden bg-background"
              style={{
                opacity: Math.max(
                  0,
                  Math.min(
                    1,
                    (inspectorVisualWidth - INSPECTOR_FADE_START)
                      / (MIN_INSPECTOR_WIDTH - INSPECTOR_FADE_START),
                  ),
                ),
              }}
            >
              <RunActInspector
                nodeId={primaryId}
                data={primaryNode?.data ?? null}
                state={primaryState ?? null}
                artifacts={primaryArtifacts}
                revealedArtifactId={revealedArtifactId}
                editable={isEditableStart}
                onPatchNode={isEditableStart
                  ? (patch) => {
                    // The start node's instruction is the run's kickoff input; the backend has no
                    // way to edit other nodes of the frozen snapshot, so description patches are
                    // intentionally ignored. Edits stay in a local draft until save.
                    if (patch.instruction != null) {
                      setInstructionDraft(patch.instruction);
                    }
                  }
                  : undefined}
                instructionDraft={isEditableStart ? instructionDraft : null}
                onInstructionDraftChange={isEditableStart ? setInstructionDraft : undefined}
                onSaveInstruction={isEditableStart ? saveInstructionDraft : undefined}
                onDiscardInstructionDraft={isEditableStart
                  ? () => setInstructionDraft(null)
                  : undefined}
                instructionSavePending={isEditableStart ? updateInput.isPending : false}
                onClose={closeInspector}
              />
            </div>
          </aside>
        </div>
      </div>
    </div>
  );
}
