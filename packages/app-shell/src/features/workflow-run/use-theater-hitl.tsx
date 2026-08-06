import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { localizeContractError } from "../../i18n/contract-error";
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
  onFocusNode: (nodeId: string) => void;
}

interface TheaterHitlController {
  openHitls: HitlRequest[];
  primaryHasHitl: boolean;
  hitlExpanded: boolean;
  /** Under-stage overlay composer (no session accessory). */
  hitlComposer: ReactNode;
  /** Embedded composer; pass the session dock so both share one chrome. */
  renderHitlComposer: (accessory?: ReactNode) => ReactNode;
  expandHitlForRequest: (requestId: string) => void;
  collapseHitl: () => void;
}

/**
 * Owns Theater HITL selection, expand/collapse, drafts, and composer mount.
 * Keeps gate chrome out of the stage layout module.
 */
export function useTheaterHitl({
  run,
  focusNodeId,
  primaryId,
  onFocusNode,
}: UseTheaterHitlParams): TheaterHitlController {
  const submitHitl = useSubmitGraphWorkflowHitl();
  const { t } = useTranslation();
  const [hitlExpanded, setHitlExpanded] = useState(false);
  const [selectedHitlId, setSelectedHitlId] = useState<string | null>(null);
  const [hitlDrafts, setHitlDrafts] = useState<Record<string, Record<string, string>>>(
    {},
  );
  const hitlEngageTimerRef = useRef<number | null>(null);

  // Theater runs swap underneath this hook, so HITL-local state (selection,
  // expansion, drafts) is reset through the documented render-adjust pattern
  // rather than a state-syncing effect.
  const [previousRunId, setPreviousRunId] = useState(run.id);
  if (previousRunId !== run.id) {
    setPreviousRunId(run.id);
    setSelectedHitlId(null);
    setHitlExpanded(false);
    setHitlDrafts({});
  }

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

  const hitlSignature = useMemo(
    () => openHitls.map((item) => item.id).sort().join("|"),
    [openHitls],
  );
  // Reconcile selection with the open gate set. A run swap counts as a fresh
  // gate set even when the signature is coincidentally identical, so the first
  // gate is picked and auto-expansion follows the stage position.
  const [previousSignature, setPreviousSignature] = useState("");
  if (previousSignature !== hitlSignature || previousRunId !== run.id) {
    const fresh = previousRunId !== run.id || previousSignature === "";
    setPreviousSignature(hitlSignature);
    if (hitlSignature === "") {
      setSelectedHitlId(null);
      setHitlExpanded(false);
      setHitlDrafts({});
    } else if (fresh) {
      setSelectedHitlId(openHitls[0]?.id ?? null);
      // Only auto-expand when the stage is already on a waiting act. If the
      // reader is on another card, keep the under-stage prompt collapsed.
      const stageOnWaitingGate = primaryId !== null
        && openHitls.some((item) => item.nodeId === primaryId);
      setHitlExpanded(stageOnWaitingGate);
    } else if (selectedHitlId === null
      || !openHitls.some((item) => item.id === selectedHitlId)) {
      setSelectedHitlId(openHitls[0]?.id ?? null);
    }
  }

  // A pending engage timer is only valid while its run/gate set is current;
  // drop it when either changes. Pure ref bookkeeping, so it stays an effect.
  useEffect(() => {
    if (hitlEngageTimerRef.current !== null) {
      window.clearTimeout(hitlEngageTimerRef.current);
      hitlEngageTimerRef.current = null;
    }
  }, [run.id, hitlSignature]);

  useEffect(() => () => {
    if (hitlEngageTimerRef.current !== null) {
      window.clearTimeout(hitlEngageTimerRef.current);
    }
  }, []);

  // Move selection to the gate under the focused act and expand the composer
  // there; browsing a non-waiting act collapses it. Keyed on focus changes via
  // the render-adjust pattern.
  const [previousFocusNodeId, setPreviousFocusNodeId] = useState<string | null>(null);
  if (previousFocusNodeId !== focusNodeId) {
    setPreviousFocusNodeId(focusNodeId);
    if (focusNodeId !== null) {
      const gate = openHitls.find((item) => item.nodeId === focusNodeId);
      if (gate !== undefined) {
        setSelectedHitlId(gate.id);
        setHitlExpanded(true);
      } else {
        // Browsing a non-waiting act — collapse to the under-stage compact
        // prompt so autofocus / engage cannot yank focus back onto the open
        // gate.
        setHitlExpanded(false);
      }
    }
  }

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

  function renderHitlComposer(accessory?: ReactNode): ReactNode {
    if (hitlGates.length === 0 || selectedHitl === null) {
      return null;
    }
    // Embedded = this act's card owns only its gate(s). Overlay (under-stage
    // while browsing elsewhere) may list every open gate so the user can jump.
    const embedded = primaryHasHitl;
    const gates = embedded && primaryId !== null
      ? hitlGates.filter((gate) => gate.request.nodeId === primaryId)
      : hitlGates;
    if (gates.length === 0) {
      return null;
    }
    const selectedRequest = gates.some((gate) => gate.request.id === selectedHitl.id)
      ? selectedHitl
      : gates[0]!.request;
    const submitError = submitHitl.error !== null
      ? localizeContractError(submitHitl.error, t)
      : null;
    return (
      <RunHitlComposer
        layout={embedded ? "embedded" : "overlay"}
        gates={gates}
        selectedRequestId={selectedRequest.id}
        onSelectRequest={expandHitlForRequest}
        expanded={hitlExpanded}
        onExpandedChange={(expanded) => {
          if (expanded) {
            expandHitlForRequest(selectedRequest.id);
            return;
          }
          collapseHitl();
        }}
        onEngage={() => {
          const requestId = selectedRequest.id;
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
          ? (submitHitl.variables?.requestId ?? selectedRequest.id)
          : null}
        submitError={submitError}
        accessory={accessory}
        onSubmit={async (payload) => {
          try {
            await submitHitl.mutateAsync({
              runId: run.id,
              requestId: selectedRequest.id,
              payload,
            });
          } catch {
            // React Query exposes the error through submitError in the composer.
          }
        }}
      />
    );
  }

  return {
    openHitls,
    primaryHasHitl,
    hitlExpanded,
    hitlComposer: renderHitlComposer(),
    renderHitlComposer,
    expandHitlForRequest,
    collapseHitl,
  };
}
