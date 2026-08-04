import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { useSubmitGraphWorkflowHitl } from "../../state/hooks/use-graph-workflow-runs";
import { RunHitlComposer } from "./run-hitl-composer";
import {
  findOpenHitlForNode,
  listOpenHitls,
  type GraphWorkflowRun,
  type HitlRequest,
} from "@ora/workflow-runtime";

interface UseTheaterHitlParams {
  run: GraphWorkflowRun;
  focusNodeId: string | null;
  primaryId: string | null;
  /** True when the parallel carousel owns the stage. */
  parallelCarouselFocus: boolean;
  onFocusNode: (nodeId: string) => void;
}

interface TheaterHitlController {
  openHitls: HitlRequest[];
  primaryHasHitl: boolean;
  hitlExpanded: boolean;
  hitlComposer: ReactNode;
  expandHitlForRequest: (requestId: string) => void;
}

/**
 * Owns Theater HITL selection, expand/collapse, drafts, and composer mount.
 * Keeps gate chrome out of the stage layout module.
 */
export function useTheaterHitl({
  run,
  focusNodeId,
  primaryId,
  parallelCarouselFocus,
  onFocusNode,
}: UseTheaterHitlParams): TheaterHitlController {
  const submitHitl = useSubmitGraphWorkflowHitl();
  const [hitlExpanded, setHitlExpanded] = useState(false);
  const [selectedHitlId, setSelectedHitlId] = useState<string | null>(null);
  const [hitlDrafts, setHitlDrafts] = useState<Record<string, Record<string, string>>>(
    {},
  );
  const hitlSignatureRef = useRef<string>("");
  const hitlEngageTimerRef = useRef<number | null>(null);
  const prevFocusHitlRef = useRef<string | null>(null);

  useEffect(() => {
    if (hitlEngageTimerRef.current !== null) {
      window.clearTimeout(hitlEngageTimerRef.current);
      hitlEngageTimerRef.current = null;
    }
    hitlSignatureRef.current = "";
    prevFocusHitlRef.current = null;
    setSelectedHitlId(null);
    setHitlExpanded(false);
    setHitlDrafts({});
  }, [run.id]);

  const openHitls = useMemo(() => listOpenHitls(run), [run]);
  const nodeTitleById = useMemo(
    () => new Map(
      run.definitionSnapshot.nodes.map((node) => [node.id, node.data.title]),
    ),
    [run.definitionSnapshot.nodes],
  );
  const hitlGates = useMemo(
    () =>
      openHitls.map((request) => ({
        request,
        nodeTitle: nodeTitleById.get(request.nodeId) ?? request.nodeId,
      })),
    [openHitls, nodeTitleById],
  );
  const selectedHitl = useMemo(() => {
    if (openHitls.length === 0) {
      return null;
    }
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

  useEffect(() => {
    const signature = openHitls.map((item) => item.id).sort().join("|");
    if (signature === "") {
      hitlSignatureRef.current = "";
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
      setHitlExpanded(true);
    }
  }, [focusNodeId, openHitls]);

  function expandHitlForRequest(requestId: string): void {
    if (hitlEngageTimerRef.current !== null) {
      window.clearTimeout(hitlEngageTimerRef.current);
      hitlEngageTimerRef.current = null;
    }
    setSelectedHitlId(requestId);
    setHitlExpanded(true);
    const gate = openHitls.find((item) => item.id === requestId);
    if (gate !== undefined) {
      onFocusNode(gate.nodeId);
    }
  }

  function collapseHitl(): void {
    if (hitlEngageTimerRef.current !== null) {
      window.clearTimeout(hitlEngageTimerRef.current);
      hitlEngageTimerRef.current = null;
    }
    setHitlExpanded(false);
  }

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
          try {
            await submitHitl.mutateAsync({
              runId: run.id,
              requestId: selectedHitl.id,
              payload,
            });
          } catch {
            // React Query exposes the error through submitError in the composer.
          }
        }}
      />
    )
    : null;

  return {
    openHitls,
    primaryHasHitl,
    hitlExpanded,
    hitlComposer,
    expandHitlForRequest,
  };
}
