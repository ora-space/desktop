import { useTranslation } from "react-i18next";
import {
  IconCheck,
  IconChevronRight,
  IconCircle,
  IconCircleDashed,
  IconLoader2,
  IconMinus,
  IconPlayerPlayFilled,
  IconX,
} from "@tabler/icons-react";
import { Button } from "@ora/ui";
import {
  getRun,
  OPTIONAL_WORKFLOW_NODES,
  suggestedNextNode,
  useWorkflowStore,
  type WorkflowNode,
  type WorkflowNodeId,
  type WorkflowNodeStatus,
} from "../../state/stores/workflow-store";
import { useWorkflowKey } from "./use-workflow-key";

interface WorkflowStepperProps {
  /** Launches a node: sends its OpenSpec reminder now, flipping it to running. */
  onLaunch: (id: WorkflowNodeId) => void;
  disabled?: boolean;
}

/**
 * The spec-driven progress strip sitting directly above the composer. Nodes fill
 * the left; the running node's actions (next, skip) and Cancel are grouped on the
 * right. Node completion is user-driven (suggest-only) and the next pending node
 * is highlighted as the recommended step rather than auto-run.
 */
export function WorkflowStepper({ onLaunch, disabled = false }: WorkflowStepperProps) {
  const { t } = useTranslation();
  const key = useWorkflowKey();
  const run = useWorkflowStore((state) => getRun(state, key));
  const completeNode = useWorkflowStore((state) => state.completeNode);
  const skipNode = useWorkflowStore((state) => state.skipNode);
  const reset = useWorkflowStore((state) => state.reset);

  if (!run.active || !run.visible) return null;

  // While a node is running, no later node is offered: the next step only lights
  // up once the user marks the current one done or skips it.
  const runningNode = run.nodes.find((node) => node.status === "running");
  const suggestedId = runningNode === undefined ? suggestedNextNode(run.nodes) : null;
  // The node in focus is the running one, or the highlighted next one. Skipping an
  // optional stage (explore, sync) is offered here even before it runs.
  const focusNode = runningNode ?? run.nodes.find((node) => node.id === suggestedId);
  const skipTargetId =
    focusNode !== undefined && OPTIONAL_WORKFLOW_NODES.has(focusNode.id) && !disabled
      ? focusNode.id
      : null;

  return (
    <div className="mb-2 flex items-center gap-1 px-1">
      <div className="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto">
        {run.nodes.map((node, index) => (
          <div key={node.id} className="flex items-center gap-1">
            {index > 0 && (
              <IconChevronRight className="size-3.5 shrink-0 text-muted-foreground/40" aria-hidden="true" />
            )}
            <StepperNode
              node={node}
              label={t(`workflow.node.${node.id}`)}
              isSuggested={node.id === suggestedId}
              canLaunch={runningNode === undefined}
              disabled={disabled}
              onLaunch={() => onLaunch(node.id)}
            />
          </div>
        ))}
      </div>
      {runningNode !== undefined && !disabled && (
        <Button
          type="button"
          variant="ghost"
          size="sm"
          onClick={() => completeNode(key, runningNode.id)}
          className="h-7 shrink-0 gap-1 rounded-md px-2 text-xs font-medium text-emerald-600 hover:bg-emerald-500/10 hover:text-emerald-600"
        >
          <IconCheck className="size-3.5" />
          {t("workflow.next")}
        </Button>
      )}
      {skipTargetId !== null && (
        <Button
          type="button"
          variant="ghost"
          size="sm"
          onClick={() => skipNode(key, skipTargetId)}
          className="h-7 shrink-0 rounded-md px-2 text-xs font-normal text-muted-foreground hover:bg-muted/60 hover:text-foreground"
        >
          {t("workflow.skip")}
        </Button>
      )}
      <Button
        type="button"
        variant="ghost"
        size="sm"
        onClick={() => reset(key)}
        aria-label={t("workflow.cancel")}
        className="h-7 shrink-0 gap-1 rounded-md px-2 text-xs font-normal text-muted-foreground hover:bg-muted/60 hover:text-foreground"
      >
        <IconX className="size-3.5" />
        {t("workflow.cancel")}
      </Button>
    </div>
  );
}

interface StepperNodeProps {
  node: WorkflowNode;
  label: string;
  isSuggested: boolean;
  /** False while another node is running, which keeps pending nodes grey and inert. */
  canLaunch: boolean;
  disabled: boolean;
  onLaunch: () => void;
}

function StepperNode({ node, label, isSuggested, canLaunch, disabled, onLaunch }: StepperNodeProps) {
  // A pending node is launchable only when nothing is running; the suggested node
  // is emphasised so the user is nudged rather than funnelled.
  const isLaunchable = node.status === "pending" && !disabled && canLaunch;

  const content = (
    <>
      <StepperIcon status={node.status} suggested={isSuggested} />
      <span
        className={
          node.status === "done"
            ? "text-emerald-600"
            : node.status === "skipped"
              ? "text-muted-foreground line-through"
              : node.status === "running"
                ? "text-sky-600"
                : isSuggested
                  ? "text-foreground"
                  : "text-muted-foreground"
        }
      >
        {label}
      </span>
    </>
  );

  if (!isLaunchable) {
    return (
      <span className="flex h-7 items-center gap-1.5 rounded-md px-2 text-xs font-normal">
        {content}
      </span>
    );
  }

  return (
    <Button
      type="button"
      variant="ghost"
      size="sm"
      onClick={onLaunch}
      className={
        isSuggested
          ? "h-7 gap-1.5 rounded-md px-2 text-xs font-medium ring-1 ring-inset ring-sky-500/30 hover:bg-sky-500/10"
          : "h-7 gap-1.5 rounded-md px-2 text-xs font-normal hover:bg-muted/60"
      }
    >
      {content}
    </Button>
  );
}

/** Status glyph, echoing plan-block's icon vocabulary so the two read as one system. */
function StepperIcon({ status, suggested }: { status: WorkflowNodeStatus; suggested: boolean }) {
  switch (status) {
    case "pending":
      return suggested ? (
        <IconPlayerPlayFilled className="size-3.5 shrink-0 text-sky-600" />
      ) : (
        <IconCircleDashed className="size-3.5 shrink-0 text-muted-foreground/60" />
      );
    case "running":
      return <IconLoader2 className="size-3.5 shrink-0 animate-spin text-sky-600 motion-reduce:animate-none" />;
    case "done":
      return <IconCheck className="size-3.5 shrink-0 text-emerald-600" />;
    case "skipped":
      return <IconMinus className="size-3.5 shrink-0 text-muted-foreground" />;
    default:
      return <IconCircle className="size-3.5 shrink-0 text-muted-foreground" />;
  }
}
