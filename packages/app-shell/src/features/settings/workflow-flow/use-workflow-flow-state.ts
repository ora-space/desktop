import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  useEdgesState,
  useNodesState,
  type Connection,
  type Edge,
  type EdgeChange,
  type NodeChange,
  type OnNodeDrag,
} from "@xyflow/react";
import type { WorkflowEdge, WorkflowNode } from "@ora/workflow-mock";
import {
  toFlowEdges,
  toFlowNodes,
  type WorkflowFlowEdge,
  type WorkflowFlowNode,
} from "./adapters";
import type { WorkflowFlowCallbacks } from "./callbacks";

interface UseWorkflowFlowStateOptions {
  nodes: WorkflowNode[];
  edges: WorkflowEdge[];
  selectedNodeId: string | null;
  onSelectNode: (nodeId: string | null) => void;
  onMoveNode: (nodeId: string, position: { x: number; y: number }) => void;
  onConnect: (source: string, target: string) => void;
  onReconnectEdge: (edgeId: string, source: string, target: string) => void;
  onDeleteNode: (nodeId: string) => void;
  onDeleteEdge: (edgeId: string) => void;
}

/** Reuses unchanged React Flow node objects so one Inspector edit only rerenders its node. */
function reconcileFlowNodes(
  current: WorkflowFlowNode[],
  next: WorkflowFlowNode[],
): WorkflowFlowNode[] {
  const currentById = new Map(current.map((node) => [node.id, node]));
  let changed = current.length !== next.length;
  const reconciled = next.map((node) => {
    const previous = currentById.get(node.id);
    if (
      previous !== undefined
      && previous.position.x === node.position.x
      && previous.position.y === node.position.y
      && previous.selected === node.selected
      && previous.data.kind === node.data.kind
      && previous.data.title === node.data.title
      && previous.data.description === node.data.description
      && previous.data.config === node.data.config
    ) {
      return previous;
    }
    changed = true;
    return node;
  });
  return changed ? reconciled : current;
}

/** Reuses unchanged edge objects while still refreshing labels affected by renamed nodes. */
function reconcileFlowEdges(
  current: WorkflowFlowEdge[],
  next: WorkflowFlowEdge[],
): WorkflowFlowEdge[] {
  const currentById = new Map(current.map((edge) => [edge.id, edge]));
  let changed = current.length !== next.length;
  const reconciled = next.map((edge) => {
    const previous = currentById.get(edge.id);
    if (
      previous !== undefined
      && previous.source === edge.source
      && previous.target === edge.target
      && previous.label === edge.label
      && previous.selected === edge.selected
      && previous.data?.sourceTitle === edge.data?.sourceTitle
      && previous.data?.targetTitle === edge.data?.targetTitle
    ) {
      return previous;
    }
    changed = true;
    return edge;
  });
  return changed ? reconciled : current;
}

/** Bridges responsive React Flow interaction state to the persisted workflow domain model. */
export function useWorkflowFlowState({
  nodes,
  edges,
  selectedNodeId,
  onSelectNode,
  onMoveNode,
  onConnect,
  onReconnectEdge,
  onDeleteNode,
  onDeleteEdge,
}: UseWorkflowFlowStateOptions) {
  const reconnectingEdgeIdRef = useRef<string | null>(null);
  const [selectedEdgeId, setSelectedEdgeId] = useState<string | null>(null);
  const [flowNodes, setFlowNodes, applyFlowNodeChanges] =
    useNodesState<WorkflowFlowNode>(toFlowNodes(nodes, selectedNodeId));
  const [flowEdges, setFlowEdges, applyFlowEdgeChanges] =
    useEdgesState(toFlowEdges(edges, nodes, selectedEdgeId));

  useEffect(() => {
    setFlowNodes((current) =>
      reconcileFlowNodes(current, toFlowNodes(nodes, selectedNodeId)),
    );
  }, [nodes, selectedNodeId, setFlowNodes]);

  useEffect(() => {
    setFlowEdges((current) =>
      reconcileFlowEdges(current, toFlowEdges(edges, nodes, selectedEdgeId)),
    );
  }, [edges, nodes, selectedEdgeId, setFlowEdges]);

  useEffect(() => {
    if (
      selectedEdgeId !== null
      && !edges.some((edge) => edge.id === selectedEdgeId)
    ) {
      setSelectedEdgeId(null);
    }
  }, [edges, selectedEdgeId]);

  /** Selects a connection while keeping node and inspector selection mutually exclusive. */
  const selectEdge = useCallback((edgeId: string) => {
    setSelectedEdgeId(edgeId);
    onSelectNode(null);
  }, [onSelectNode]);

  /** Deletes a connection and clears its local selection immediately. */
  const deleteEdge = useCallback((edgeId: string) => {
    setSelectedEdgeId((current) => (current === edgeId ? null : current));
    onDeleteEdge(edgeId);
  }, [onDeleteEdge]);

  const flowCallbacks = useMemo<WorkflowFlowCallbacks>(
    () => ({
      onDeleteNode,
      onDeleteEdge: deleteEdge,
      onSelectEdge: selectEdge,
    }),
    [deleteEdge, onDeleteNode, selectEdge],
  );

  /** Applies transient React Flow changes locally so dragging does not rerender the settings page. */
  const handleNodesChange = useCallback((changes: NodeChange<WorkflowFlowNode>[]) => {
    applyFlowNodeChanges(changes);
    for (const change of changes) {
      if (change.type === "remove") {
        onDeleteNode(change.id);
      }
    }
  }, [applyFlowNodeChanges, onDeleteNode]);

  /** Applies edge selection and reconnect visuals locally while forwarding domain deletions. */
  const handleEdgesChange = useCallback((changes: EdgeChange[]) => {
    applyFlowEdgeChanges(changes);
    for (const change of changes) {
      if (change.type === "remove") {
        deleteEdge(change.id);
      }
    }
  }, [applyFlowEdgeChanges, deleteEdge]);

  /** Commits one final position after a drag, avoiding dirty-state writes on every pointer move. */
  const handleNodeDragStop = useCallback<OnNodeDrag<WorkflowFlowNode>>(
    (_event, node) => {
      onMoveNode(node.id, node.position);
    },
    [onMoveNode],
  );

  /** Rejects self-loops and duplicate directed pairs, excluding the edge being reconnected. */
  const isValidConnection = useCallback(
    (connection: Connection | Edge) => {
      const source = connection.source;
      const target = connection.target;
      if (source === null || target === null || source === target) {
        return false;
      }
      const reconnectingEdgeId = reconnectingEdgeIdRef.current;
      return !edges.some(
        (edge) =>
          edge.id !== reconnectingEdgeId
          && edge.source === source
          && edge.target === target,
      );
    },
    [edges],
  );

  /** Creates a domain edge after React Flow has accepted both connection endpoints. */
  const handleConnect = useCallback((connection: Connection) => {
    if (connection.source !== null && connection.target !== null) {
      onConnect(connection.source, connection.target);
    }
  }, [onConnect]);

  /** Commits a valid reconnect gesture to the domain graph. */
  const handleReconnect = useCallback((oldEdge: Edge, connection: Connection) => {
    if (connection.source !== null && connection.target !== null) {
      onReconnectEdge(oldEdge.id, connection.source, connection.target);
    }
  }, [onReconnectEdge]);

  /** Clears both local edge selection and parent-owned node selection. */
  const clearSelection = useCallback(() => {
    setSelectedEdgeId(null);
    onSelectNode(null);
  }, [onSelectNode]);

  return {
    clearSelection,
    deleteEdge,
    flowCallbacks,
    flowEdges,
    flowNodes,
    handleConnect,
    handleEdgesChange,
    handleNodeDragStop,
    handleNodesChange,
    handleReconnect,
    isValidConnection,
    reconnectingEdgeIdRef,
    selectEdge,
    setSelectedEdgeId,
  };
}
