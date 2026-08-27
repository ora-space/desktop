import {
  getBezierPath,
  useReactFlow,
  type ConnectionLineComponentProps,
} from "@xyflow/react";
import {
  WORKFLOW_NODE_ANCHOR_Y,
  WORKFLOW_NODE_WIDTH,
} from "@ora/workflow-mock";
import { useWorkflowConnectionState } from "./use-connection-state";

/** Uses the same soft curve for a connection preview as for a committed edge. */
export function WorkflowConnectionLine({
  fromX,
  fromY,
  toX,
  toY,
  fromPosition,
  toPosition,
  connectionLineStyle,
  connectionStatus,
}: ConnectionLineComponentProps) {
  const { connectionCandidateEndpoint, connectionCandidateNodeId } =
    useWorkflowConnectionState();
  const { getInternalNode } = useReactFlow();
  const candidateNode =
    connectionCandidateNodeId === null ||
    connectionCandidateNodeId === undefined
      ? undefined
      : getInternalNode(connectionCandidateNodeId);
  const target =
    candidateNode !== undefined &&
    connectionCandidateEndpoint !== null &&
    connectionCandidateEndpoint !== undefined
      ? {
          x:
            candidateNode.internals.positionAbsolute.x +
            (connectionCandidateEndpoint === "source"
              ? WORKFLOW_NODE_WIDTH
              : 0),
          y:
            candidateNode.internals.positionAbsolute.y + WORKFLOW_NODE_ANCHOR_Y,
        }
      : { x: toX, y: toY };
  const [path] = getBezierPath({
    sourceX: fromX,
    sourceY: fromY,
    sourcePosition: fromPosition,
    targetX: target.x,
    targetY: target.y,
    targetPosition: toPosition,
  });

  return (
    <path
      d={path}
      fill="none"
      className="workflow-connection-preview"
      style={connectionLineStyle}
      data-status={connectionStatus ?? undefined}
    />
  );
}
