import { useTranslation } from "react-i18next";
import type { RunPathRegionPhase } from "./run-path-structure";

interface RunTheaterRegionContextProps {
  regionTitle: string;
  nodeTitle: string;
  round: number;
  roundCount: number;
  phaseKind: RunPathRegionPhase["kind"];
  peerIndex: number;
  peerCount: number;
}

/** Keeps the selected member's region, round, and branch context above its detail card. */
export function RunTheaterRegionContext({
  regionTitle,
  nodeTitle,
  round,
  roundCount,
  phaseKind,
  peerIndex,
  peerCount,
}: RunTheaterRegionContextProps) {
  const { t } = useTranslation();
  const phaseLabel =
    phaseKind === "parallel"
      ? t("workflowRun.theater.contextParallel", {
          index: peerIndex + 1,
          count: peerCount,
        })
      : phaseKind === "conditional"
        ? t("workflowRun.theater.contextConditional", {
            index: peerIndex + 1,
            count: peerCount,
          })
        : t("workflowRun.theater.contextSequential");

  return (
    <nav
      className="mb-2 flex min-w-0 items-center gap-1.5 overflow-hidden px-1 text-[11px] leading-snug text-muted-foreground"
      aria-label={t("workflowRun.theater.executionContext")}
    >
      <span className="truncate">{regionTitle}</span>
      <span aria-hidden>/</span>
      <span className="shrink-0">
        {t("workflowRun.theater.roundPosition", {
          round: round + 1,
          total: roundCount,
        })}
      </span>
      <span aria-hidden>/</span>
      <span className="shrink-0">{phaseLabel}</span>
      <span aria-hidden>/</span>
      <span className="truncate font-medium text-foreground">{nodeTitle}</span>
    </nav>
  );
}
