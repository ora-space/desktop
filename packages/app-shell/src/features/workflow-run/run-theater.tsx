import { isTerminalRunStatus } from "@ora/workflow-runtime";
import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
} from "react";
import { useTranslation } from "react-i18next";
import { Badge, cn, toast } from "@ora/ui";
import { useUpdateWorkflowRunInput } from "../../state/data/workflow-runs";
import { filterArtifacts, latestArtifact } from "./artifact-filter";
import {
  projectLoopRoundNodeStates,
  selectedLoopRound,
  type LoopRoundSelection,
} from "./loop-round-state";
import { RunActInspector } from "./run-act-inspector";
import { RunResultAct } from "./run-result-act";
import { RunTheaterActCard } from "./run-theater-act-card";
import { RunTheaterParallelStage } from "./run-theater-parallel-stage";
import { RunTheaterPathRail } from "./run-theater-path-rail";
import { RunTheaterRegionContext } from "./run-theater-region-context";
import {
  projectRunPathStructure,
  type RunPathRegionStage,
} from "./run-path-structure";
import { resolveTheaterFocus } from "./run-focus";
import { isNodeWorking } from "./run-status-style";
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
   * mount, artifact reveal). Intentional node-detail actions still open the rail so
   * Diff and the inspector can coexist on the stage.
   */
  reviewPanelOpen?: boolean;
  /** Result act CTA — return to Overview path map. */
  onShowOverview: () => void;
  /** Which node's session dock is open — lifted across Overview remounts. */
  sessionConversationNodeId?: string | null;
  onSessionConversationNodeIdChange?: (nodeId: string | null) => void;
  /** Reports a completed interactive node so the workspace can follow its successor. */
  onNodeCompleted?: (nodeId: string) => void;
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
  onNodeCompleted,
}: RunTheaterProps) {
  const { t } = useTranslation();
  const updateInput = useUpdateWorkflowRunInput();
  // Local draft of the Start input while the user edits it. Committed to the run's
  // kickoff input only by the explicit save action, so per-keystroke refetches cannot clobber
  // an in-progress edit or fire one mutation per character.
  const [instructionDraft, setInstructionDraft] = useState<string | null>(null);
  /** Theater round viewer: the round of the focused region node being inspected. */
  const [selectedRound, setSelectedRound] = useState<number | null>(null);
  const [variableDraft, setVariableDraft] = useState<Record<
    string,
    unknown
  > | null>(null);
  const inspectorAnimationRef = useRef<number | null>(null);
  const inspectorWidthRef = useRef(DEFAULT_INSPECTOR_WIDTH);
  const inspectorCurrentWidthRef = useRef(0);
  const resizeDragRef = useRef<{ startX: number; startWidth: number } | null>(
    null,
  );
  const [inspectorCollapsed, setInspectorCollapsed] = useState(true);
  const [inspectorVisualWidth, setInspectorVisualWidth] = useState(0);
  const pathScrollOpenSigRef = useRef<string>("");
  const pathRailRef = useRef<HTMLDivElement | null>(null);
  const [loopRoundSelection, setLoopRoundSelection] =
    useState<LoopRoundSelection>({});
  const [loopRoundSelectionRunId, setLoopRoundSelectionRunId] = useState(
    run.id,
  );
  if (loopRoundSelectionRunId !== run.id) {
    setLoopRoundSelectionRunId(run.id);
    setLoopRoundSelection({});
  }

  const visibleNodeStates = useMemo(
    () => projectLoopRoundNodeStates(run, loopRoundSelection),
    [run, loopRoundSelection],
  );
  const visibleRun = useMemo(
    () => ({ ...run, nodeStates: visibleNodeStates }),
    [run, visibleNodeStates],
  );
  const nodeById = useMemo(
    () => new Map(run.definitionSnapshot.nodes.map((node) => [node.id, node])),
    [run.definitionSnapshot.nodes],
  );
  const pathStructure = useMemo(
    () => projectRunPathStructure(run.definitionSnapshot),
    [run.definitionSnapshot],
  );

  const focus = useMemo(
    () => resolveTheaterFocus(visibleRun, focusNodeId),
    [visibleRun, focusNodeId],
  );
  const primaryId = focus.primaryId;
  const primaryNode = primaryId === null ? undefined : nodeById.get(primaryId);
  const primaryRegionId =
    (primaryNode?.parentId !== undefined &&
    nodeById.get(primaryNode.parentId)?.data.kind === "iteration"
      ? primaryNode.parentId
      : undefined) ??
    (primaryNode?.data.kind === "iteration" ? primaryNode.id : null);
  const primaryRegion =
    primaryNode?.parentId === undefined
      ? null
      : (pathStructure.find(
          (stage): stage is RunPathRegionStage =>
            stage.type === "region" && stage.nodeId === primaryNode.parentId,
        ) ?? null);
  const primaryRegionNode =
    primaryRegion === null ? undefined : nodeById.get(primaryRegion.nodeId);
  const primaryRegionPhase = primaryRegion?.phases.find((phase) =>
    phase.nodeIds.includes(primaryId ?? ""),
  );
  const primaryRegionPeerIndex =
    primaryRegionPhase?.nodeIds.indexOf(primaryId ?? "") ?? -1;
  const parallel = focus.activeIds.length > 1;
  const parallelCarouselFocus =
    primaryId !== null &&
    parallel &&
    focus.activeIds.length > 1 &&
    focus.activeIds.includes(primaryId) &&
    primaryNode?.parentId === undefined;
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
    run: visibleRun,
    focusNodeId,
    primaryId,
    onFocusNode,
  });

  // Scroll path rail when the open-gate set changes — not on every primary tick.
  useEffect(() => {
    const requestSig = openHitls
      .map((item) => item.id)
      .sort()
      .join("|");
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

  const primaryState =
    primaryId !== null ? visibleNodeStates[primaryId] : undefined;
  const primaryRounds =
    primaryId !== null ? (run.roundStates?.[primaryId] ?? []) : [];
  const regionRoundNumbers = useMemo(() => {
    if (primaryRegionId === null) return [];
    const rounds = new Set<number>();
    for (const node of run.definitionSnapshot.nodes) {
      if (node.parentId !== primaryRegionId) continue;
      for (const state of run.roundStates?.[node.id] ?? []) {
        if (state.iteration !== undefined) rounds.add(state.iteration);
      }
    }
    return [...rounds].sort((left, right) => left - right);
  }, [primaryRegionId, run.definitionSnapshot.nodes, run.roundStates]);
  // Round selection belongs to the region, not an individual member. Sibling switches therefore
  // preserve the round while leaving/re-entering the region resumes automatic latest-round follow.
  const regionSelectionKey = `${run.id}:${primaryRegionId ?? "outer"}`;
  const [roundRegionKey, setRoundRegionKey] = useState(regionSelectionKey);
  if (regionSelectionKey !== roundRegionKey) {
    setRoundRegionKey(regionSelectionKey);
    setSelectedRound(null);
  }
  const regionSelectedRound =
    regionSelectionKey === roundRegionKey ? selectedRound : null;
  const effectiveRegionRound =
    primaryRegionId === null
      ? null
      : (regionSelectedRound ??
        regionRoundNumbers[regionRoundNumbers.length - 1] ??
        null);
  const selectedPrimaryRound =
    primaryNode?.parentId !== undefined && effectiveRegionRound !== null
      ? primaryRounds.find((round) => round.iteration === effectiveRegionRound)
      : undefined;
  const primaryDisplayState =
    primaryNode?.parentId !== undefined && effectiveRegionRound !== null
      ? (selectedPrimaryRound ?? {
          status: "inactive" as const,
          iteration: effectiveRegionRound,
        })
      : primaryState;
  // The Start input is editable whenever the run is not executing — a not-started pending
  // run or any terminal run — so the kickoff input can be changed before a restart re-runs it.
  const isEditableStart =
    (run.status === "pending" || isTerminalRunStatus(run.status)) &&
    primaryNode?.data?.kind === "start";

  // Drop an uncommitted draft the moment the run leaves pending (or the run switches) so a stale
  // draft cannot reappear on the start node after a restart. Implemented as a render-time reset
  // keyed on the last pending lifecycle state instead of an effect to avoid a cascading render.
  const [draftPendingKey, setDraftPendingKey] = useState("");
  const nextDraftPendingKey = run.status === "pending" ? run.id : "";
  if (nextDraftPendingKey !== draftPendingKey) {
    setDraftPendingKey(nextDraftPendingKey);
    setInstructionDraft(null);
    setVariableDraft(null);
  }
  const primaryArtifacts = useMemo(
    () =>
      primaryId === null
        ? []
        : filterArtifacts(artifacts, { type: "node", nodeId: primaryId }),
    [artifacts, primaryId],
  );
  const primaryRealConversation = primaryDisplayState?.conversation;
  const primaryConversation = useMemo(() => {
    if (primaryNode?.parentId !== undefined && effectiveRegionRound !== null) {
      // Region history is round-scoped. Falling back to the node-level projection here would
      // silently show another round's session when this member did not execute in the selection.
      return primaryRealConversation ?? [];
    }
    // The real adapter projects the node's conversation from its run output; the mock
    // runtime provides it through the live snapshot instead.
    const mockItems =
      primaryId === null ? [] : (conversationByNodeId.get(primaryId) ?? []);
    return primaryRealConversation != null && primaryRealConversation.length > 0
      ? primaryRealConversation
      : mockItems;
  }, [
    primaryId,
    primaryNode?.parentId,
    effectiveRegionRound,
    conversationByNodeId,
    primaryRealConversation,
  ]);
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
      const state = visibleNodeStates[nodeId];
      if (node === undefined || state === undefined) {
        return [];
      }
      return [
        {
          nodeId,
          data: node.data,
          state,
          artifactCount: artifactCountByNode[nodeId] ?? 0,
          conversation:
            state.conversation != null && state.conversation.length > 0
              ? state.conversation
              : (conversationByNodeId.get(nodeId) ?? []),
        },
      ];
    });
  }, [
    parallel,
    focus.activeIds,
    nodeById,
    visibleNodeStates,
    artifactCountByNode,
    conversationByNodeId,
  ]);

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
   * open; explicit user actions always open so the rail can sit beside an open Diff.
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

  /** Uses the card's persistent header control as the single inspector toggle. */
  function toggleInspector(): void {
    if (inspectorCollapsed) {
      openInspector("user");
      return;
    }
    closeInspector();
  }

  // Commits the drafted Start input to the run's kickoff input in one request and
  // clears the draft on success (the refetched run then surfaces the saved input).
  function saveInstructionDraft(): void {
    if (instructionDraft === null && variableDraft === null) {
      return;
    }
    updateInput.mutate(
      {
        runId: run.id,
        input: instructionDraft ?? primaryNode?.data.input ?? "",
        ...(variableDraft === null ? {} : { variables: variableDraft }),
      },
      {
        onSuccess: () => {
          setInstructionDraft(null);
          setVariableDraft(null);
        },
        onError: () => toast.error(t("workflowRun.updateFailed")),
      },
    );
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
      targetWidth:
        width < INSPECTOR_COLLAPSE_THRESHOLD ? 0 : MIN_INSPECTOR_WIDTH,
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
      revealedArtifactId === null ||
      showResultAct ||
      sessionConversationNodeId !== null
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

  const primaryConversationOpen =
    primaryNode?.data.kind === "agent" &&
    sessionConversationNodeId === primaryNode.id;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <RunTheaterPathRail
        run={visibleRun}
        primaryId={primaryId}
        activeIds={focus.activeIds}
        openHitls={openHitls}
        artifactCountByNode={artifactCountByNode}
        showResultAct={showResultAct}
        selectedRound={effectiveRegionRound}
        onRoundChange={setSelectedRound}
        pathRailRef={pathRailRef}
        onFocusNode={onFocusNode}
        onExpandHitl={expandHitlForRequest}
        onShowResultAct={
          isTerminalRunStatus(run.status) ? onClearFocus : undefined
        }
      />

      <div className="relative flex min-h-0 flex-1 flex-col overflow-hidden">
        <div className="relative min-h-0 flex-1 overflow-hidden">
          <div
            className={cn(
              "absolute inset-0 flex flex-col p-6",
              primaryConversationOpen ? "overflow-hidden" : "overflow-auto",
            )}
            style={{ right: inspectorVisualWidth }}
          >
            <div
              className="pointer-events-none absolute inset-0 bg-[radial-gradient(ellipse_at_50%_30%,color-mix(in_oklch,var(--muted)_55%,transparent),transparent_65%)]"
              aria-hidden
            />
            <div
              className={cn(
                "relative w-full",
                primaryConversationOpen
                  ? "flex h-full min-h-0 max-w-none flex-1 flex-col"
                  : "mx-auto my-auto max-w-xl shrink-0",
              )}
            >
              {showResultAct ? (
                <RunResultAct
                  run={run}
                  artifactCount={artifacts.length}
                  changedFileCount={changedFileCount}
                  onShowOverview={onShowOverview}
                  onOpenArtifacts={
                    artifacts.length > 0
                      ? () => {
                          const recent = latestArtifact(artifacts);
                          if (recent !== null) {
                            onFocusNode(recent.nodeId);
                          }
                          openInspector("user");
                        }
                      : undefined
                  }
                />
              ) : showParallelCarousel && !primaryConversationOpen ? (
                <div className="space-y-3">
                  <RunTheaterParallelStage
                    runId={run.id}
                    acts={parallelActs}
                    primaryId={primaryId!}
                    onFocusNode={onFocusNode}
                    inspectorOpen={!inspectorCollapsed}
                    onToggleInspector={toggleInspector}
                    sessionConversationNodeId={sessionConversationNodeId}
                    onSessionConversationNodeIdChange={
                      onSessionConversationNodeIdChange
                    }
                    onNodeCompleted={onNodeCompleted}
                    primaryInteraction={
                      primaryHasHitl
                        ? ({ accessory }) =>
                            renderHitlComposer(accessory ?? undefined)
                        : undefined
                    }
                  />
                  {!primaryHasHitl && hitlComposer !== null && (
                    <div className="px-0.5">{hitlComposer}</div>
                  )}
                </div>
              ) : primaryNode && primaryDisplayState ? (
                <div
                  key={primaryNode.id}
                  className={cn(
                    "animate-in fade-in zoom-in-95 slide-in-from-bottom-2 duration-300 ease-[cubic-bezier(0.22,1,0.36,1)] fill-mode-both motion-reduce:animate-none",
                    primaryConversationOpen &&
                      "flex h-full min-h-0 w-full flex-1 flex-col overflow-hidden",
                  )}
                >
                  {primaryRegion !== null &&
                    primaryRegionNode !== undefined &&
                    primaryRegionPhase !== undefined &&
                    primaryRegionPeerIndex >= 0 &&
                    effectiveRegionRound !== null && (
                      <RunTheaterRegionContext
                        regionTitle={primaryRegionNode.data.title}
                        nodeTitle={primaryNode.data.title}
                        round={effectiveRegionRound}
                        roundCount={regionRoundNumbers.length}
                        phaseKind={primaryRegionPhase.kind}
                        peerIndex={primaryRegionPeerIndex}
                        peerCount={primaryRegionPhase.nodeIds.length}
                      />
                    )}
                  <RunTheaterActCard
                    data={primaryNode.data}
                    state={primaryDisplayState}
                    runId={run.id}
                    nodeId={primaryNode.id}
                    live={isNodeWorking(primaryDisplayState.status)}
                    artifactCount={primaryArtifacts.length}
                    conversation={primaryConversation}
                    conversationEnabled={primaryNode.data.kind === "agent"}
                    conversationOpen={
                      sessionConversationNodeId === primaryNode.id
                    }
                    onConversationOpenChange={(open) => {
                      onSessionConversationNodeIdChange?.(
                        open ? primaryNode.id : null,
                      );
                    }}
                    onNodeCompleted={onNodeCompleted}
                    variant="stage"
                    inspectorOpen={!inspectorCollapsed}
                    onToggleInspector={toggleInspector}
                    interaction={
                      primaryHasHitl
                        ? ({ accessory }) =>
                            renderHitlComposer(accessory ?? undefined)
                        : undefined
                    }
                  />
                  {!primaryHasHitl && hitlComposer !== null && (
                    <div className="mt-3">{hitlComposer}</div>
                  )}
                </div>
              ) : (
                <p className="text-center text-sm text-muted-foreground">
                  {t("workflowRun.theater.empty")}
                </p>
              )}

              {!showResultAct && !primaryConversationOpen && (
                <div className="mt-6 flex flex-wrap items-center justify-center gap-2">
                  {parallel && primaryNode?.parentId === undefined && (
                    <Badge variant="secondary" className="tabular-nums">
                      {t("workflowRun.theater.parallelCount", {
                        count: focus.activeIds.length,
                      })}
                    </Badge>
                  )}
                </div>
              )}
              {!showResultAct && !primaryConversationOpen && (
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
                    (inspectorVisualWidth - INSPECTOR_FADE_START) /
                      (MIN_INSPECTOR_WIDTH - INSPECTOR_FADE_START),
                  ),
                ),
              }}
            >
              <RunActInspector
                roundStates={run.roundStates}
                selectedRound={effectiveRegionRound}
                onRoundChange={setSelectedRound}
                showRoundSelector={primaryNode?.parentId === undefined}
                nodeId={primaryId}
                data={primaryNode?.data ?? null}
                state={primaryDisplayState ?? null}
                artifacts={primaryArtifacts}
                revealedArtifactId={revealedArtifactId}
                runStatus={run.status}
                runSnapshotId={run.snapshotId}
                loopRounds={
                  primaryNode?.data.kind === "loop"
                    ? (run.rounds ?? []).filter(
                        (round) => round.parentLoopNodeId === primaryNode.id,
                      )
                    : undefined
                }
                loopChildTitles={
                  primaryNode?.data.kind === "loop"
                    ? Object.fromEntries(
                        run.definitionSnapshot.nodes
                          .filter(
                            (node) => node.data.containerId === primaryNode.id,
                          )
                          .map((node) => [node.id, node.data.title]),
                      )
                    : undefined
                }
                selectedLoopRoundId={
                  primaryNode?.data.kind === "loop"
                    ? selectedLoopRound(
                        run.rounds ?? [],
                        primaryNode.id,
                        loopRoundSelection,
                      )?.id
                    : undefined
                }
                onSelectedLoopRoundChange={
                  primaryNode?.data.kind === "loop"
                    ? (roundId) =>
                        setLoopRoundSelection((current) => ({
                          ...current,
                          [primaryNode.id]: roundId,
                        }))
                    : undefined
                }
                editable={isEditableStart}
                onPatchNode={
                  isEditableStart
                    ? (patch) => {
                        // The start node's input is the run's kickoff input; the backend has no
                        // way to edit other nodes of the frozen snapshot, so description patches are
                        // intentionally ignored. Edits stay in a local draft until save.
                        if (patch.input != null) {
                          setInstructionDraft(patch.input);
                        }
                      }
                    : undefined
                }
                instructionDraft={isEditableStart ? instructionDraft : null}
                variableDraft={isEditableStart ? variableDraft : null}
                onInstructionDraftChange={
                  isEditableStart ? setInstructionDraft : undefined
                }
                onVariableDraftChange={
                  isEditableStart
                    ? (name, value) =>
                        setVariableDraft((current) => ({
                          ...(current ?? {}),
                          [name]: value,
                        }))
                    : undefined
                }
                onSaveInstruction={
                  isEditableStart ? saveInstructionDraft : undefined
                }
                onDiscardInstructionDraft={
                  isEditableStart
                    ? () => {
                        setInstructionDraft(null);
                        setVariableDraft(null);
                      }
                    : undefined
                }
                instructionSavePending={
                  isEditableStart ? updateInput.isPending : false
                }
                onClose={primaryNode === undefined ? closeInspector : undefined}
              />
            </div>
          </aside>
        </div>
      </div>
    </div>
  );
}
