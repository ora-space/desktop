// scripts/repair_workflow_snapshots.mjs
import path from "node:path";
import { DatabaseSync } from "node:sqlite";

const dbPath = path.join(
  process.env.APPDATA,
  "space.ora.desktop",
  "ora.sqlite3",
);
console.log("Opening SQLite database at:", dbPath);

const db = new DatabaseSync(dbPath);

export function repairGraph(graph) {
  if (!graph || !Array.isArray(graph.nodes)) return graph;

  const nodes = graph.nodes;
  const edges = Array.isArray(graph.edges) ? graph.edges : [];

  // 1. Identify container kinds
  const loopIds = new Set(
    nodes.filter((n) => n.data?.kind === "loop").map((n) => n.id),
  );
  const iterIds = new Set(
    nodes.filter((n) => n.data?.kind === "iteration").map((n) => n.id),
  );
  const containerIds = new Set([...loopIds, ...iterIds]);

  const containerChildren = new Map();
  for (const cId of containerIds) {
    containerChildren.set(cId, []);
  }

  const outerNodes = [];
  for (const node of nodes) {
    if (!node.data) node.data = {};

    // Normalize agentConfig
    if (node.data.kind === "agent") {
      if (!node.data.agentConfig) {
        node.data.agentConfig = {
          schemaVersion: 3,
          executor: {
            agentCli: "official/ora-space.opencode",
            modelId: "bluezone/zhipu/glm-5.3",
          },
          roleId: "",
          skills: [],
          mcps: [],
          prompt: "",
          interactive: false,
        };
      }
      if (!Array.isArray(node.data.agentConfig.skills)) {
        node.data.agentConfig.skills = [];
      }
      if (!Array.isArray(node.data.agentConfig.mcps)) {
        node.data.agentConfig.mcps = [];
      }
      if (typeof node.data.agentConfig.roleId !== "string") {
        node.data.agentConfig.roleId = "";
      }
      if (typeof node.data.agentConfig.prompt !== "string") {
        node.data.agentConfig.prompt = "";
      }
    }

    const parentId = node.parentId || node.data.containerId;
    if (parentId && loopIds.has(parentId)) {
      node.parentId = parentId;
      node.data.containerId = parentId; // Loop children MUST have containerId
      containerChildren.get(parentId).push(node);
    } else if (parentId && iterIds.has(parentId)) {
      node.parentId = parentId;
      delete node.data.containerId; // CRITICAL: Iteration children MUST NOT have containerId!
      containerChildren.get(parentId).push(node);
    } else {
      delete node.parentId;
      if (node.data?.containerId) delete node.data.containerId;
      outerNodes.push(node);
    }
  }

  // 2. Position container children relative to container
  for (const [cId, children] of containerChildren.entries()) {
    const containerNode = nodes.find((n) => n.id === cId);
    let curX = 40;
    for (let i = 0; i < children.length; i++) {
      const child = children[i];
      if (!child.position || typeof child.position.x !== "number") {
        child.position = { x: curX, y: 120 };
      }
      curX += 280;
    }
    const neededWidth = Math.max(680, curX + 60);
    const neededHeight = 380;
    if (
      !containerNode.initialWidth ||
      containerNode.initialWidth < neededWidth
    ) {
      containerNode.initialWidth = neededWidth;
    }
    if (
      !containerNode.initialHeight ||
      containerNode.initialHeight < neededHeight
    ) {
      containerNode.initialHeight = neededHeight;
    }
  }

  // 3. Compute DAG rank for outer nodes
  const outerNodeIds = new Set(outerNodes.map((n) => n.id));
  const outerEdges = edges.filter(
    (e) => outerNodeIds.has(e.source) && outerNodeIds.has(e.target),
  );

  const indegree = new Map(outerNodes.map((n) => [n.id, 0]));
  const outgoing = new Map(outerNodes.map((n) => [n.id, []]));

  for (const edge of outerEdges) {
    outgoing.get(edge.source)?.push(edge.target);
    indegree.set(edge.target, (indegree.get(edge.target) || 0) + 1);
  }

  const rank = new Map(outerNodes.map((n) => [n.id, 0]));
  const queue = outerNodes
    .filter((n) => indegree.get(n.id) === 0)
    .map((n) => n.id);

  while (queue.length > 0) {
    const curr = queue.shift();
    const currRank = rank.get(curr) || 0;
    for (const next of outgoing.get(curr) || []) {
      const existingRank = rank.get(next) || 0;
      if (currRank + 1 > existingRank) {
        rank.set(next, currRank + 1);
      }
      const newIn = (indegree.get(next) || 1) - 1;
      indegree.set(next, newIn);
      if (newIn <= 0) {
        queue.push(next);
      }
    }
  }

  // Group outer nodes by rank
  const rankGroups = new Map();
  for (const node of outerNodes) {
    const r = rank.get(node.id) || 0;
    if (!rankGroups.has(r)) rankGroups.set(r, []);
    rankGroups.get(r).push(node);
  }

  let curOuterX = 80;
  const sortedRanks = Array.from(rankGroups.keys()).sort((a, b) => a - b);
  for (const r of sortedRanks) {
    const group = rankGroups.get(r);
    let maxColWidth = 260;
    for (let i = 0; i < group.length; i++) {
      const node = group[i];
      const nodeWidth =
        node.initialWidth || (node.data?.kind === "condition" ? 320 : 240);
      if (nodeWidth > maxColWidth) maxColWidth = nodeWidth;

      if (!node.position || typeof node.position.x !== "number") {
        const yOffset = (i - (group.length - 1) / 2) * 180;
        node.position = {
          x: curOuterX,
          y: Math.round(200 + yOffset),
        };
      }
    }
    curOuterX += maxColWidth + 100;
  }

  // 4. Ensure all nodes have type="workflow"
  for (const node of nodes) {
    node.type = "workflow";
    if (!node.position || typeof node.position.x !== "number") {
      node.position = { x: 100, y: 100 };
    }
  }

  // 5. Ensure all edges have id and type="workflow"
  for (let i = 0; i < edges.length; i++) {
    const edge = edges[i];
    edge.type = "workflow";
    if (!edge.id) {
      edge.id = `e-${edge.source}-${edge.target}${edge.sourceHandle ? "-" + edge.sourceHandle : ""}-${i}`;
    }
  }

  if (loopIds.size > 0 || iterIds.size > 0) {
    graph.schemaVersion = 2;
  }
  if (!graph.viewport) {
    graph.viewport = { x: 0, y: 0, zoom: 1 };
  }
  if (!Array.isArray(graph.annotations)) {
    graph.annotations = [];
  }
  if (!Array.isArray(graph.globalVariables)) {
    graph.globalVariables = [];
  }

  return graph;
}

const rows = db
  .prepare("SELECT id, graph FROM workflow_snapshots WHERE is_deleted = 0")
  .all();
console.log(
  `Found ${rows.length} total active snapshots to inspect and repair.`,
);

let updatedCount = 0;
const updateStmt = db.prepare(
  "UPDATE workflow_snapshots SET graph = ? WHERE id = ?",
);

for (const row of rows) {
  try {
    const parsed = JSON.parse(row.graph);
    const updated = repairGraph(parsed);
    const updatedJson = JSON.stringify(updated);
    if (updatedJson !== row.graph) {
      updateStmt.run(updatedJson, row.id);
      updatedCount++;
    }
  } catch (err) {
    console.error(`Failed to repair snapshot ${row.id}:`, err);
  }
}

console.log(`Successfully repaired and updated ${updatedCount} snapshots.`);

// PRAGMA checkpoint
db.exec("PRAGMA wal_checkpoint(TRUNCATE);");
console.log("WAL checkpoint completed.");
