import type {
  WorkflowDefinition,
  WorkflowDefinitionEdge,
  WorkflowDefinitionNode,
  WorkflowNodeData,
} from "./types";
import { validateWorkflowDefinition } from "./definition";

/** Inputs available when planning a mock path (extensible for HITL / schemas later). */
export interface MockExecutionContext {
  kickoffInput?: string;
}

type WorkflowEdge = WorkflowDefinitionEdge;
type WorkflowNode = WorkflowDefinitionNode;

/**
 * Pluggable path selection for exclusive condition fan-out.
 * Non-condition nodes still follow every outgoing edge (fan-in/fan-out DAG).
 */
export interface MockPathPolicy {
  chooseConditionEdge: (
    outgoing: WorkflowEdge[],
    node: WorkflowNode,
    context: MockExecutionContext,
  ) => WorkflowEdge;
}

export interface MockExecutionPlan {
  /** Reachable nodes in topological order (documentation / tests). */
  order: string[];
  /** Unreachable on the chosen path; engine marks these skipped. */
  skipped: string[];
  /**
   * Predecessors inside the reachable subgraph.
   * The engine starts a node only when every predecessor has succeeded.
   */
  predecessors: Record<string, string[]>;
}

/** Deterministic default: kickoff-aware label heuristics, else first outgoing edge. */
export function createDefaultMockPathPolicy(): MockPathPolicy {
  return {
    chooseConditionEdge(outgoing, _node, context) {
      if (outgoing.length === 0) {
        throw new Error("Condition node has no outgoing edges");
      }
      const input = (context.kickoffInput ?? "").toLowerCase();
      const labelOf = (edge: WorkflowEdge) => String(edge.label ?? "").toLowerCase();

      // Doc-only style kickoff prefers the documentation branch when labels match.
      if (/doc|readme|markdown|\.md|文档|说明|注释/.test(input)) {
        const docEdge = outgoing.find((edge) =>
          /文档|doc|only|readme|markdown/.test(labelOf(edge)),
        );
        if (docEdge !== undefined) {
          return docEdge;
        }
      }

      // Code/test kickoff prefers the validation branch when labels match.
      if (/test|code|src|spec|实现|测试|代码|检查/.test(input)) {
        const checkEdge = outgoing.find((edge) =>
          /检查|test|check|验证|需要/.test(labelOf(edge)),
        );
        if (checkEdge !== undefined) {
          return checkEdge;
        }
      }

      return outgoing[0]!;
    },
  };
}

/**
 * Walks the graph from start seeds, applying the path policy at condition nodes.
 * Returns a topo order over the reachable subgraph and the skipped remainder.
 */
export function planMockExecution(
  workflow: WorkflowDefinition,
  context: MockExecutionContext = {},
  policy: MockPathPolicy = createDefaultMockPathPolicy(),
): MockExecutionPlan {
  validateWorkflowDefinition(workflow);
  const nodes = workflow.nodes as WorkflowNode[];
  const edges = workflow.edges as WorkflowEdge[];
  const ids = nodes.map((node) => node.id);
  const idSet = new Set(ids);
  const nodeById = new Map(nodes.map((node) => [node.id, node]));
  const outgoing = new Map<string, WorkflowEdge[]>(ids.map((id) => [id, []]));
  const indegree = new Map(ids.map((id) => [id, 0]));

  for (const edge of edges) {
    if (!idSet.has(edge.source) || !idSet.has(edge.target)) {
      continue;
    }
    outgoing.get(edge.source)!.push(edge);
    indegree.set(edge.target, (indegree.get(edge.target) ?? 0) + 1);
  }

  const seeds = nodes
    .filter((node) => node.data.kind === "start")
    .map((node) => node.id);
  const startIds = seeds.length > 0
    ? seeds
    : ids.filter((id) => (indegree.get(id) ?? 0) === 0);

  const reachable = new Set<string>();
  const walk: string[] = [...startIds];
  while (walk.length > 0) {
    const id = walk.shift()!;
    if (reachable.has(id)) {
      continue;
    }
    reachable.add(id);
    const node = nodeById.get(id);
    const outs = outgoing.get(id) ?? [];
    if (node?.data.kind === "condition") {
      if (outs.length > 0) {
        const chosen = policy.chooseConditionEdge(outs, node, context);
        if (!outs.includes(chosen)) {
          throw new Error(`Condition policy returned an unrelated edge for node ${id}`);
        }
        walk.push(chosen.target);
      }
      continue;
    }
    for (const edge of outs) {
      walk.push(edge.target);
    }
  }

  const skipped = ids.filter((id) => !reachable.has(id));
  const reachableEdges = edges.filter(
    (edge) => reachable.has(edge.source) && reachable.has(edge.target),
  );
  const order = topologicalOrder([...reachable], reachableEdges);
  const predecessors: Record<string, string[]> = {};
  for (const id of order) {
    predecessors[id] = [];
  }
  for (const edge of reachableEdges) {
    predecessors[edge.target]?.push(edge.source);
  }
  return { order, skipped, predecessors };
}

/** Kahn order over an arbitrary id subset; appends leftovers for cycles. */
export function topologicalOrder(
  ids: string[],
  edges: WorkflowEdge[],
): string[] {
  const idSet = new Set(ids);
  const indegree = new Map(ids.map((id) => [id, 0]));
  const adjacency = new Map(ids.map((id) => [id, [] as string[]]));

  for (const edge of edges) {
    if (!idSet.has(edge.source) || !idSet.has(edge.target)) {
      continue;
    }
    adjacency.get(edge.source)!.push(edge.target);
    indegree.set(edge.target, (indegree.get(edge.target) ?? 0) + 1);
  }

  const queue = ids.filter((id) => (indegree.get(id) ?? 0) === 0);
  const order: string[] = [];
  while (queue.length > 0) {
    const id = queue.shift()!;
    order.push(id);
    for (const next of adjacency.get(id) ?? []) {
      const nextDegree = (indegree.get(next) ?? 1) - 1;
      indegree.set(next, nextDegree);
      if (nextDegree === 0) {
        queue.push(next);
      }
    }
  }

  for (const id of ids) {
    if (!order.includes(id)) {
      order.push(id);
    }
  }
  return order;
}

/** Kinds that participate in mock token accounting (LLM/tool work). */
export function nodeKindUsesTokens(kind: WorkflowNodeData["kind"]): boolean {
  return kind === "prompt" || kind === "agent" || kind === "tool";
}
