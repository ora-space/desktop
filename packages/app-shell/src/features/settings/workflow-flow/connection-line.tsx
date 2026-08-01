import {
  getBezierPath,
  useReactFlow,
  type ConnectionLineComponentProps,
} from "@xyflow/react";
import { workflowConnectionAnchor } from "./path";
import { NODE_ANCHOR_Y, NODE_WIDTH } from "./layout";
import { useWorkflowConnectionState } from "./connection-state";

/** Uses the same soft curve for a connection preview as for a committed edge. */
export function WorkflowConnectionLine({
  fromX,
  fromY,
  toX,
  toY,
  fromHandle,
  toHandle,
  fromPosition,
  toPosition,
  connectionLineStyle,
  connectionStatus,
}: ConnectionLineComponentProps) {
  const {
    connectionCandidateEndpoint,
    connectionCandidateNodeId,
  } = useWorkflowConnectionState();
  const { getInternalNode } = useReactFlow();
  const source = workflowConnectionAnchor({
    x: fromX,
    y: fromY,
    position: fromPosition,
    width: fromHandle.width,
    height: fromHandle.height,
  });
  const candidateNode = connectionCandidateNodeId === null
    || connectionCandidateNodeId === undefined
    ? undefined
    : getInternalNode(connectionCandidateNodeId);
  const target = candidateNode !== undefined
    && connectionCandidateEndpoint !== null
    && connectionCandidateEndpoint !== undefined
    ? {
        x: candidateNode.internals.positionAbsolute.x
          + (connectionCandidateEndpoint === "source" ? NODE_WIDTH : 0),
        y: candidateNode.internals.positionAbsolute.y + NODE_ANCHOR_Y,
      }
    : toHandle === null
      ? { x: toX, y: toY }
      : workflowConnectionAnchor({
        x: toX,
        y: toY,
        position: toPosition,
        width: toHandle.width,
        height: toHandle.height,
      });
  const [path] = getBezierPath({
    sourceX: source.x,
    sourceY: source.y,
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
