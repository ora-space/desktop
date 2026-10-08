/** Prevents layout repair from inventing executable identity or changing scope ownership. */
function isRecord(value) {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** IDs and owners must retain their meaning for selectors and frozen run references. */
function isIdentity(value) {
  return typeof value === "string" && value.trim() !== "";
}

/** A single valid coordinate is insufficient for React Flow's geometry calculations. */
function isPosition(value) {
  return (
    isRecord(value) && Number.isFinite(value.x) && Number.isFinite(value.y)
  );
}

/** Invalid sizes must not propagate NaN or string arithmetic into container layout. */
function isDimension(value) {
  return typeof value === "number" && Number.isFinite(value) && value > 0;
}

/** Validates topology without guessing whether malformed children were meant to be root nodes. */
function validateStructure(graph) {
  if (
    !isRecord(graph) ||
    !Array.isArray(graph.nodes) ||
    !Array.isArray(graph.edges)
  ) {
    throw new Error("Workflow graph requires node and edge arrays");
  }
  const byId = new Map();
  for (const [index, node] of graph.nodes.entries()) {
    if (
      !isRecord(node) ||
      !isIdentity(node.id) ||
      !isRecord(node.data) ||
      !isIdentity(node.data.kind)
    ) {
      throw new Error(`Node ${index} requires an ID and data.kind`);
    }
    if (byId.has(node.id)) throw new Error(`Duplicate node ID ${node.id}`);
    byId.set(node.id, node);
  }
  const owners = new Map();
  for (const node of graph.nodes) {
    const container = node.data.containerId;
    const parent = node.parentId ?? undefined;
    if (container !== undefined && !isIdentity(container)) {
      throw new Error(`Node ${node.id} has an invalid containerId`);
    }
    if (parent !== undefined && !isIdentity(parent)) {
      throw new Error(`Node ${node.id} has an invalid parentId`);
    }
    if (
      container !== undefined &&
      parent !== undefined &&
      container !== parent
    ) {
      throw new Error(`Node ${node.id} has conflicting container ownership`);
    }
    const ownerId = container ?? parent;
    const owner = byId.get(ownerId);
    if (ownerId !== undefined) {
      if (!owner)
        throw new Error(`Node ${node.id} has unknown parent ${ownerId}`);
      const expectedKind = container === undefined ? "iteration" : "loop";
      if (owner.data.kind !== expectedKind) {
        throw new Error(
          `Node ${node.id} has invalid ${expectedKind} ownership`,
        );
      }
      if (
        owner.parentId != null ||
        owner.data.containerId !== undefined ||
        ["loop", "iteration"].includes(node.data.kind)
      ) {
        throw new Error(`Node ${node.id} has nested container ownership`);
      }
    }
    owners.set(node.id, ownerId);
  }
  for (const [index, edge] of graph.edges.entries()) {
    if (
      !isRecord(edge) ||
      !isIdentity(edge.source) ||
      !isIdentity(edge.target)
    ) {
      throw new Error(`Edge ${index} requires source and target node IDs`);
    }
    if (!byId.has(edge.source) || !byId.has(edge.target)) {
      throw new Error(`Edge ${index} refers to an unknown node`);
    }
    const sourceOwner = owners.get(edge.source);
    const targetOwner = owners.get(edge.target);
    if (
      sourceOwner !== targetOwner &&
      !(
        byId.get(edge.source).data.kind === "iteration" &&
        targetOwner === edge.source
      )
    ) {
      throw new Error(`Edge ${index} crosses a container boundary`);
    }
  }
  return owners;
}

/** Positions missing geometry by DAG columns, with a bounded fallback for authoring cycles. */
function layoutScope(nodes, edges, origin) {
  const byId = new Map(nodes.map((node) => [node.id, node]));
  const incoming = new Map(nodes.map((node) => [node.id, 0]));
  const outgoing = new Map(nodes.map((node) => [node.id, []]));
  const ranks = new Map(nodes.map((node) => [node.id, 0]));
  for (const edge of edges) {
    if (!byId.has(edge.source) || !byId.has(edge.target)) continue;
    outgoing.get(edge.source).push(edge.target);
    incoming.set(edge.target, incoming.get(edge.target) + 1);
  }
  const queue = nodes
    .filter((node) => incoming.get(node.id) === 0)
    .map((node) => node.id);
  for (let index = 0; index < queue.length; index++) {
    const current = queue[index];
    for (const target of outgoing.get(current)) {
      ranks.set(target, Math.max(ranks.get(target), ranks.get(current) + 1));
      incoming.set(target, incoming.get(target) - 1);
      if (incoming.get(target) === 0) queue.push(target);
    }
  }
  const groups = new Map();
  for (const node of nodes) {
    const rank = ranks.get(node.id);
    if (!groups.has(rank)) groups.set(rank, []);
    groups.get(rank).push(node);
  }
  const occupied = nodes.filter((node) => isPosition(node.position));
  let x = origin.x;
  for (const rank of [...groups.keys()].sort((left, right) => left - right)) {
    const group = groups.get(rank);
    let y = origin.y;
    let columnWidth = 240;
    for (const node of group) {
      const width =
        node.initialWidth ?? (node.data.kind === "condition" ? 320 : 240);
      const height = node.initialHeight ?? 120;
      columnWidth = Math.max(columnWidth, width);
      if (!isPosition(node.position)) {
        // Existing author positions stay intact; repaired nodes must not obscure them.
        let collision;
        do {
          collision = occupied.find((other) => {
            const otherWidth =
              other.initialWidth ??
              (other.data.kind === "condition" ? 320 : 240);
            const otherHeight = other.initialHeight ?? 120;
            return (
              x < other.position.x + otherWidth + 40 &&
              x + width + 40 > other.position.x &&
              y < other.position.y + otherHeight + 40 &&
              y + height + 40 > other.position.y
            );
          });
          if (collision)
            y = collision.position.y + (collision.initialHeight ?? 120) + 60;
        } while (collision);
        node.position = { x, y };
        occupied.push(node);
      }
      y = Math.max(y, node.position.y + height + 60);
    }
    x += columnWidth + 100;
  }
}

/**
 * Repairs only display geometry on a detached graph. Domain payloads, dependency selections,
 * executable edges, and declared ownership stay unchanged; ambiguous structure is rejected.
 */
export function layoutWorkflowGraph(graph) {
  const owners = validateStructure(graph);
  const repaired = structuredClone(graph);
  const containers = repaired.nodes.filter((node) =>
    ["loop", "iteration"].includes(node.data.kind),
  );
  if (containers.length > 0) {
    if (
      graph.schemaVersion !== undefined &&
      graph.schemaVersion !== 1 &&
      graph.schemaVersion !== 2
    ) {
      throw new Error("Container graph has an unsupported schemaVersion");
    }
    repaired.schemaVersion = 2;
  }
  for (const node of repaired.nodes) {
    node.type = "workflow";
    for (const key of ["width", "height", "initialWidth", "initialHeight"]) {
      if (node[key] !== undefined && !isDimension(node[key])) delete node[key];
    }
    // containerId is the domain owner for Loops; deriving its visual parent is unambiguous.
    if (node.data.containerId !== undefined && node.parentId == null) {
      node.parentId = node.data.containerId;
    }
  }
  for (const container of containers) {
    const children = repaired.nodes.filter(
      (node) => owners.get(node.id) === container.id,
    );
    layoutScope(children, repaired.edges, { x: 40, y: 120 });
    const right = Math.max(
      0,
      ...children.map(
        (child) => child.position.x + (child.initialWidth ?? 240),
      ),
    );
    const bottom = Math.max(
      0,
      ...children.map(
        (child) => child.position.y + (child.initialHeight ?? 120),
      ),
    );
    container.initialWidth = Math.max(
      container.initialWidth ?? container.width ?? 0,
      680,
      right + 60,
    );
    container.initialHeight = Math.max(
      container.initialHeight ?? container.height ?? 0,
      380,
      bottom + 60,
    );
  }
  layoutScope(
    repaired.nodes.filter((node) => owners.get(node.id) === undefined),
    repaired.edges,
    { x: 80, y: 80 },
  );
  const reserved = new Set(
    repaired.edges.map((edge) => edge.id).filter(isIdentity),
  );
  const seen = new Set();
  for (const [index, edge] of repaired.edges.entries()) {
    edge.type = "workflow";
    if (!isIdentity(edge.id) || seen.has(edge.id)) {
      const base = `workflow-edge-${index + 1}`;
      let id = base;
      let suffix = 2;
      while (reserved.has(id)) id = `${base}-${suffix++}`;
      edge.id = id;
      reserved.add(id);
    }
    seen.add(edge.id);
  }
  if (!isPosition(repaired.viewport) || !isDimension(repaired.viewport.zoom)) {
    repaired.viewport = { x: 0, y: 0, zoom: 1 };
  }
  return repaired;
}
