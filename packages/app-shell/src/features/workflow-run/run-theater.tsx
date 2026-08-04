import { useEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import { useTranslation } from "react-i18next";
import { Badge, cn } from "@ora/ui";
import { useUpdateGraphWorkflowRunSnapshotNode, useSubmitGraphWorkflowHitl } from "../../state/hooks/use-graph-workflow-runs";
import { filterArtifacts, latestArtifact } from "./artifact-filter";
import { RunActInspector } from "./run-act-inspector";
import { RunHitlComposer } from "./run-hitl-composer";
import { RunResultAct } from "./run-result-act";
import { RunTheaterActCard } from "./run-theater-act-card";
import { RunTheaterParallelStage } from "./run-theater-parallel-stage";
import { resolveTheaterFocus } from "./run-focus";
import { RunStatusMark, isNodeWorking } from "./run-status-mark";
import { isTerminalRunStatus, runStatusTone } from "./run-status-style";
import {
  animateOverlayWidth,
  cancelOverlayWidthAnimation,
} from "./theater-overlay-motion";
import type { GraphWorkflowRun, WorkflowArtifact } from "./runtime/types";
import { findOpenHitlForNode, listOpenHitls } from "./runtime/types";
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
  artifacts: WorkflowArtifact[];
  revealedArtifactId: string | null;
  /** Opens the companion rail once on mount (e.g. Overview → Theater via node click). */
  openInspectorOnMount?: boolean;
  /** Result act CTA — return to Overview path map. */
  onShowOverview: () => void;
}

/**
 * Focused act stage + path rail + overlay companion inspector.
 * Stage stays full-bleed (card does not shift) when the rail opens.
 * HITL embeds in the waiting act card or docks under the stage / carousel.
 * Terminal + no path pin → result act. Esc (workspace) returns to Overview.
 */
export function RunTheater({
  run,
  focusNodeId,
  onFocusNode,
  artifacts,
  revealedArtifactId,
  openInspectorOnMount = false,
  onShowOverview,
}: RunTheaterProps) {
  const { t } = useTranslation();
  const updateSnapshotNode = useUpdateGraphWorkflowRunSnapshotNode();
  const submitHitl = useSubmitGraphWorkflowHitl();
  const inspectorAnimationRef = useRef<number | null>(null);
  const inspectorWidthRef = useRef(DEFAULT_INSPECTOR_WIDTH);
  const inspectorCurrentWidthRef = useRef(0);
  const resizeDragRef = useRef<{ startX: number; startWidth: number } | null>(null);
  const [inspectorCollapsed, setInspectorCollapsed] = useState(true);
  const [inspectorVisualWidth, setInspectorVisualWidth] = useState(0);
  const [hitlExpanded, setHitlExpanded] = useState(false);
  const [selectedHitlId, setSelectedHitlId] = useState<string | null>(null);
  const [hitlDrafts, setHitlDrafts] = useState<Record<string, Record<string, string>>>(
    {},
  );
  const hitlSignatureRef = useRef<string>("");
  const hitlEngageTimerRef = useRef<number | null>(null);
  const hitlUserCollapsedRef = useRef(false);
  const prevFocusHitlRef = useRef<string | null>(null);
  const pathScrollOpenSigRef = useRef<string>("");
  const pathRailRef = useRef<HTMLDivElement | null>(null);

  const focus = useMemo(
    () => resolveTheaterFocus(run, focusNodeId),
    [run, focusNodeId],
  );
  const primaryId = focus.primaryId;
  const parallel = focus.activeIds.length > 1;
  const openHitls = useMemo(() => listOpenHitls(run), [run]);
  const hitlGates = useMemo(
    () =>
      openHitls.map((request) => ({
        request,
        nodeTitle: run.definitionSnapshot.nodes.find((node) => node.id === request.nodeId)
          ?.data.title ?? request.nodeId,
      })),
    [openHitls, run.definitionSnapshot.nodes],
  );
  const selectedHitl = useMemo(() => {
    if (openHitls.length === 0) {
      return null;
    }
    // Stage waiting act owns the form when it has an open gate (avoids A/B mismatch).
    if (primaryId !== null) {
      const primaryGate = findOpenHitlForNode(run, primaryId);
      if (primaryGate !== undefined) {
        return primaryGate;
      }
    }
    const focused = focusNodeId !== null
      ? findOpenHitlForNode(run, focusNodeId)
      : undefined;
    if (focused !== undefined) {
      return focused;
    }
    if (selectedHitlId !== null) {
      const picked = openHitls.find((item) => item.id === selectedHitlId);
      if (picked !== undefined) {
        return picked;
      }
    }
    return openHitls[0] ?? null;
  }, [openHitls, focusNodeId, run, selectedHitlId, primaryId]);

  // Keep dock collapsed when the open-gate set first appears; switching
  // between already-open gates should not force-collapse the form.
  useEffect(() => {
    const signature = openHitls.map((item) => item.id).sort().join("|");
    if (signature === "") {
      hitlSignatureRef.current = "";
      hitlUserCollapsedRef.current = false;
      if (hitlEngageTimerRef.current !== null) {
        window.clearTimeout(hitlEngageTimerRef.current);
        hitlEngageTimerRef.current = null;
      }
      setSelectedHitlId(null);
      setHitlExpanded(false);
      setHitlDrafts({});
      return;
    }
    if (hitlSignatureRef.current === "") {
      hitlSignatureRef.current = signature;
      setSelectedHitlId(openHitls[0]?.id ?? null);
      setHitlExpanded(false);
      return;
    }
    if (hitlSignatureRef.current !== signature) {
      hitlSignatureRef.current = signature;
      if (selectedHitlId === null || !openHitls.some((item) => item.id === selectedHitlId)) {
        setSelectedHitlId(openHitls[0]?.id ?? null);
      }
    }
  }, [openHitls, selectedHitlId]);

  useEffect(() => () => {
    if (hitlEngageTimerRef.current !== null) {
      window.clearTimeout(hitlEngageTimerRef.current);
    }
  }, []);

  // Focusing a waiting act selects that gate; expand only when focus *changes*
  // so run ticks cannot undo a user collapse while browsing.
  useEffect(() => {
    if (focusNodeId === null) {
      prevFocusHitlRef.current = null;
      return;
    }
    const gate = openHitls.find((item) => item.nodeId === focusNodeId);
    if (gate === undefined) {
      prevFocusHitlRef.current = focusNodeId;
      return;
    }
    const focusChanged = prevFocusHitlRef.current !== focusNodeId;
    prevFocusHitlRef.current = focusNodeId;
    setSelectedHitlId(gate.id);
    if (focusChanged) {
      hitlUserCollapsedRef.current = false;
      setHitlExpanded(true);
    }
  }, [focusNodeId, openHitls]);

  // Scroll path rail when the open-gate set changes — not on every primary tick.
  useEffect(() => {
    const openSig = openHitls.map((item) => item.id).sort().join("|");
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
  }, [openHitls, primaryId]);

  /** Opens the HITL surface and spotlights that gate’s act on stage. */
  function expandHitlForRequest(requestId: string): void {
    if (hitlEngageTimerRef.current !== null) {
      window.clearTimeout(hitlEngageTimerRef.current);
      hitlEngageTimerRef.current = null;
    }
    hitlUserCollapsedRef.current = false;
    setSelectedHitlId(requestId);
    setHitlExpanded(true);
    const gate = openHitls.find((item) => item.id === requestId);
    if (gate !== undefined) {
      onFocusNode(gate.nodeId);
    }
  }

  /** Collapse without a pending overlay engage undoing the dismiss. */
  function collapseHitl(): void {
    if (hitlEngageTimerRef.current !== null) {
      window.clearTimeout(hitlEngageTimerRef.current);
      hitlEngageTimerRef.current = null;
    }
    hitlUserCollapsedRef.current = true;
    setHitlExpanded(false);
  }

  // Esc collapses HITL first (capture), so Overview exit waits for a second Esc.
  useEffect(() => {
    function onKeyDown(event: KeyboardEvent): void {
      if (event.key !== "Escape" || !hitlExpanded) {
        return;
      }
      event.preventDefault();
      event.stopImmediatePropagation();
      collapseHitl();
    }
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [hitlExpanded]);

  const primaryHitl = primaryId !== null
    ? findOpenHitlForNode(run, primaryId)
    : undefined;
  const primaryHasHitl = primaryHitl !== undefined;
  // Same condition as showParallelCarousel below — used early for HITL layout.
  const parallelCarouselFocus = primaryId !== null
    && parallel
    && focus.activeIds.length > 1
    && focus.activeIds.includes(primaryId);

  const hitlComposer = hitlGates.length > 0 && selectedHitl !== null
    ? (
      <RunHitlComposer
        layout={parallelCarouselFocus || !primaryHasHitl ? "overlay" : "embedded"}
        gates={hitlGates}
        selectedRequestId={selectedHitl.id}
        onSelectRequest={expandHitlForRequest}
        expanded={hitlExpanded}
        onExpandedChange={(expanded) => {
          if (expanded) {
            expandHitlForRequest(selectedHitl.id);
            return;
          }
          collapseHitl();
        }}
        onEngage={() => {
          const requestId = selectedHitl.id;
          if (hitlEngageTimerRef.current !== null) {
            window.clearTimeout(hitlEngageTimerRef.current);
          }
          hitlEngageTimerRef.current = window.setTimeout(() => {
            hitlEngageTimerRef.current = null;
            expandHitlForRequest(requestId);
          }, 0);
        }}
        drafts={hitlDrafts}
        onDraftsChange={setHitlDrafts}
        submitting={submitHitl.isPending}
        submittingRequestId={submitHitl.isPending
          ? (submitHitl.variables?.requestId ?? selectedHitl.id)
          : null}
        submitError={submitHitl.error instanceof Error
          ? submitHitl.error.message
          : null}
        onSubmit={async (payload) => {
          await submitHitl.mutateAsync({
            runId: run.id,
            requestId: selectedHitl.id,
            payload,
          });
        }}
      />
    )
    : null;

  const primaryNode = run.definitionSnapshot.nodes.find(
    (node) => node.id === primaryId,
  );
  const primaryState = primaryId !== null
    ? run.nodeStates[primaryId]
    : undefined;
  const primaryArtifacts = useMemo(
    () =>
      primaryId === null
        ? []
        : filterArtifacts(artifacts, { type: "node", nodeId: primaryId }),
    [artifacts, primaryId],
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
      const node = run.definitionSnapshot.nodes.find((item) => item.id === nodeId);
      const state = run.nodeStates[nodeId];
      if (node === undefined || state === undefined) {
        return [];
      }
      return [{
        nodeId,
        data: node.data,
        state,
        artifactCount: artifactCountByNode[nodeId] ?? 0,
      }];
    });
  }, [
    parallel,
    focus.activeIds,
    run.definitionSnapshot.nodes,
    run.nodeStates,
    artifactCountByNode,
  ]);

  // Parallel carousel only when the focused act is one of the live peers.
  // Otherwise show a single card so path chips can open completed / pending nodes
  // while HITL gates are still open elsewhere.
  const showParallelCarousel = parallelCarouselFocus;
  // Terminal + no history pin → result act (path chips still open single-node review).
  const showResultAct = isTerminalRunStatus(run.status) && focusNodeId === null;

  const progress = useMemo(() => {
    const states = Object.values(run.nodeStates);
    const total = Math.max(states.length, 1);
    const done = states.filter(
      (state) =>
        state.status === "succeeded"
        || state.status === "skipped"
        || state.status === "failed"
        || state.status === "cancelled",
    ).length;
    return { done, total, percent: Math.round((done / total) * 100) };
  }, [run.nodeStates]);

  useEffect(() => {
    return () => cancelOverlayWidthAnimation(inspectorAnimationRef);
  }, []);

  /** Applies overlay width and remembers a stable open size. */
  function applyInspectorWidth(width: number): void {
    const next = Math.max(0, Math.min(MAX_INSPECTOR_WIDTH, width));
    inspectorCurrentWidthRef.current = next;
    setInspectorVisualWidth(next);
    setInspectorCollapsed(next < 1);
    if (next >= MIN_INSPECTOR_WIDTH) {
      inspectorWidthRef.current = next;
    }
  }

  /** Opens or settles the overlay inspector to the last remembered width. */
  function openInspector(): void {
    // Don't cover an embedded HITL form; parallel docks HITL below the carousel.
    if (hitlExpanded && primaryHasHitl && !showParallelCarousel) {
      return;
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

  /** Fully collapses the overlay inspector (stage stays put). */
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

  /** Snaps an undersized inspector only after pointer release. */
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
    // Dragging the left edge rightward shrinks the overlay.
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

  // Overview → Theater via node click: reveal details without a second click.
  useEffect(() => {
    if (!openInspectorOnMount) {
      return;
    }
    openInspector();
    // Mount-only: Theater remounts when switching back from Overview.
    // eslint-disable-next-line react-hooks/exhaustive-deps -- intentional
  }, []);

  // New outcome: open the companion rail so reveal motion is visible.
  // Skip on the result act — keep that surface uncluttered until the user asks.
  useEffect(() => {
    if (revealedArtifactId === null || showResultAct) {
      return;
    }
    openInspector();
  }, [revealedArtifactId, showResultAct]);

  // Landing on the result act closes a leftover rail from the last live tick.
  useEffect(() => {
    if (!showResultAct || inspectorCollapsed) {
      return;
    }
    closeInspector();
    // Only react to entering the result surface; ignore later collapse toggles.
    // eslint-disable-next-line react-hooks/exhaustive-deps -- intentional
  }, [showResultAct]);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="shrink-0 border-b border-border/80 bg-muted/20 px-4 py-3">
        <div className="mx-auto flex max-w-3xl flex-col gap-2.5">
          <div className="flex items-center justify-between gap-3">
            <p className="text-[11px] font-medium uppercase tracking-[0.05em] text-muted-foreground">
              {t("workflowRun.theater.path")}
            </p>
            <p className="text-[11px] tabular-nums text-muted-foreground">
              {t("workflowRun.progressValue", {
                done: progress.done,
                total: progress.total,
              })}
            </p>
          </div>
          <div
            className="h-1.5 overflow-hidden rounded-full bg-muted"
            role="progressbar"
            aria-valuenow={progress.percent}
            aria-valuemin={0}
            aria-valuemax={100}
            aria-label={t("workflowRun.field.progress")}
          >
            <div
              className={cn(
                "relative h-full overflow-hidden rounded-full bg-foreground/75 transition-[width] duration-500 ease-[cubic-bezier(0.22,1,0.36,1)] motion-reduce:transition-none",
                run.status === "running" && "bg-sky-600/80",
                run.status === "awaiting_input" && "bg-amber-600/80",
                run.status === "succeeded" && "bg-emerald-600/75",
                (run.status === "failed" || run.status === "partial_failed")
                  && "bg-rose-600/75",
                run.status === "cancelled" && "bg-zinc-500/60",
              )}
              style={{ width: `${progress.percent}%` }}
            >
              {(run.status === "running" || run.status === "awaiting_input") && (
                <span className="theater-progress-sheen absolute inset-0" aria-hidden />
              )}
            </div>
          </div>
          <div className="overflow-x-auto" ref={pathRailRef} data-slot="theater-path-rail">
            <ol className="flex w-max gap-2 pb-0.5">
              {run.definitionSnapshot.nodes.map((node) => {
                const state = run.nodeStates[node.id] ?? { status: "idle" as const };
                const tone = runStatusTone(state.status);
                const selected = !showResultAct && node.id === primaryId;
                const waiting = state.status === "awaiting_input";
                const active = focus.activeIds.includes(node.id);
                const nodeArtifactCount = artifactCountByNode[node.id] ?? 0;
                return (
                  <li key={node.id}>
                    <button
                      type="button"
                      data-path-node={node.id}
                      data-waiting={waiting ? "" : undefined}
                      onClick={() => {
                        const gate = openHitls.find((item) => item.nodeId === node.id);
                        if (gate !== undefined) {
                          expandHitlForRequest(gate.id);
                          return;
                        }
                        onFocusNode(node.id);
                      }}
                      className={cn(
                        "inline-flex max-w-[11rem] cursor-pointer items-center gap-2 rounded-full border px-2.5 py-1.5 text-left transition-[transform,colors,box-shadow] duration-200",
                        selected && waiting
                          ? "theater-chip-pop border-amber-500/55 bg-amber-500/15 text-amber-950 shadow-sm dark:text-amber-50"
                          : selected
                          ? "theater-chip-pop border-foreground/35 bg-background shadow-sm"
                          : waiting
                          ? "border-amber-500/40 bg-amber-500/10 text-amber-950 dark:text-amber-100"
                          : active
                          ? "border-sky-500/40 bg-sky-500/10"
                          : "border-transparent bg-background/60 hover:border-border hover:bg-background",
                      )}
                      aria-current={selected ? "step" : undefined}
                      aria-label={`${node.data.title}: ${t(tone.labelKey)}`}
                    >
                      <RunStatusMark status={state.status} quiet />
                      <span className="truncate text-[11px] font-medium">
                        {node.data.title}
                      </span>
                      {nodeArtifactCount > 0 && (
                        <span
                          className="shrink-0 tabular-nums text-[9px] text-muted-foreground"
                          aria-label={t("workflowRun.artifacts.countBadge", {
                            count: nodeArtifactCount,
                          })}
                        >
                          {nodeArtifactCount}
                        </span>
                      )}
                    </button>
                  </li>
                );
              })}
            </ol>
          </div>
        </div>
      </div>

      <div className="relative flex min-h-0 flex-1 flex-col overflow-hidden">
        <div className="relative min-h-0 flex-1 overflow-hidden">
          <div className="absolute inset-0 flex flex-col overflow-auto p-6">
            <div
              className="pointer-events-none absolute inset-0 bg-[radial-gradient(ellipse_at_50%_30%,color-mix(in_oklch,var(--muted)_55%,transparent),transparent_65%)]"
              aria-hidden
            />
            {/*
              my-auto centers when the column is short; when content overflows,
              auto margins collapse so the column starts at the top and scrolls
              without covering the path rail above this pane.
            */}
            <div className="relative mx-auto my-auto w-full max-w-xl shrink-0">
              {showResultAct
                ? (
                  <RunResultAct
                    run={run}
                    artifactCount={artifacts.length}
                    onShowOverview={onShowOverview}
                    onOpenArtifacts={artifacts.length > 0
                      ? () => {
                        const recent = latestArtifact(artifacts);
                        if (recent !== null) {
                          // Pin the producing act so the rail matches the count CTA.
                          onFocusNode(recent.nodeId);
                        }
                        openInspector();
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
                      onOpenInspector={openInspector}
                    />
                    {/*
                      Keep HITL under the carousel (not inside sliding cards) so
                      peer switches don’t remount a tall form mid-slide.
                    */}
                    {hitlComposer !== null && (
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
                      nodeId={primaryNode.id}
                      data={primaryNode.data}
                      state={primaryState}
                      live={isNodeWorking(primaryState.status)}
                      artifactCount={primaryArtifacts.length}
                      variant="stage"
                      onSelect={openInspector}
                      interaction={primaryHasHitl ? hitlComposer : undefined}
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
                {run.totals.tokenUsage?.totalTokens !== undefined && (
                  <Badge variant="secondary" className="tabular-nums">
                    {t("workflowRun.totalsTokens", {
                      count: run.totals.tokenUsage.totalTokens,
                    })}
                  </Badge>
                )}
                {run.totals.durationMs !== undefined && (
                  <Badge variant="secondary" className="tabular-nums">
                    {t("workflowRun.totalsDuration", {
                      ms: run.totals.durationMs,
                    })}
                  </Badge>
                )}
              </div>
              )}
              {!showResultAct && (
              <p className="mt-3 text-center text-[10px] text-muted-foreground/70">
                {inspectorCollapsed
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
              editable={run.status === "pending"}
              onPatchNode={run.status === "pending" && primaryId !== null
                ? (patch) => {
                  void updateSnapshotNode.mutateAsync({
                    runId: run.id,
                    nodeId: primaryId,
                    patch,
                  });
                }
                : undefined}
              onClose={closeInspector}
            />
          </div>
        </aside>
        </div>
      </div>
    </div>
  );
}
