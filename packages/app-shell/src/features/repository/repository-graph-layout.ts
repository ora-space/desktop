import type { RepositoryCommit } from "@ora/contracts";

export const REPOSITORY_GRAPH_ROW_HEIGHT = 72;
export const REPOSITORY_GRAPH_LANE_WIDTH = 20;
export const REPOSITORY_GRAPH_NODE_SIZE = 18;
export const REPOSITORY_GRAPH_LANE_COLORS = [
  "#0ea5e9",
  "#a855f7",
  "#f97316",
  "#22c55e",
  "#f43f5e",
  "#eab308",
  "#14b8a6",
  "#8b5cf6",
] as const;

export interface RepositoryGraphParentLayout {
  parentId: string;
  parentRowIndex: number | null;
  laneIndex: number;
  colorIndex: number;
}

export interface RepositoryGraphRowLayout {
  commit: RepositoryCommit;
  laneIndex: number;
  laneColorIndex: number;
  laneCount: number;
  parents: RepositoryGraphParentLayout[];
}

export interface RepositoryGraphLayout {
  rows: RepositoryGraphRowLayout[];
  laneCount: number;
}

interface ActiveGraphLane {
  commitId: string;
  colorIndex: number;
}

/** Creates a compact, deterministic author marker until an external avatar can be resolved. */
export function getRepositoryAuthorInitials(authorName: string, authorEmail: string): string {
  const source = authorName.trim() || authorEmail.split("@")[0]?.trim() || "?";
  const parts = source.split(/\s+/).filter(Boolean);
  if (parts.length >= 2) {
    return `${parts[0][0] ?? ""}${parts.at(-1)?.[0] ?? ""}`.toLocaleUpperCase();
  }

  return Array.from(source).slice(0, 2).join("").toLocaleUpperCase();
}

/** Computes stable lane ownership so the renderer can connect nodes across arbitrary row distances. */
export function buildRepositoryGraphLayout(commits: RepositoryCommit[]): RepositoryGraphLayout {
  const lanes: ActiveGraphLane[] = [];
  let nextColorIndex = 0;
  const rows = commits.map((commit) => {
    let laneIndex = lanes.findIndex((lane) => lane.commitId === commit.id);
    if (laneIndex === -1) {
      laneIndex = 0;
      lanes.unshift({ commitId: commit.id, colorIndex: nextColorIndex });
      nextColorIndex += 1;
    }

    const currentLane = lanes[laneIndex];
    lanes.splice(laneIndex, 1);

    commit.parents.forEach((parentId, parentIndex) => {
      if (lanes.some((lane) => lane.commitId === parentId)) {
        return;
      }

      const insertionIndex = Math.min(laneIndex + parentIndex, lanes.length);
      lanes.splice(insertionIndex, 0, {
        commitId: parentId,
        colorIndex: parentIndex === 0 ? currentLane.colorIndex : nextColorIndex,
      });
      if (parentIndex > 0) {
        nextColorIndex += 1;
      }
    });

    const parents = commit.parents.map((parentId) => {
      const parentLaneIndex = lanes.findIndex((lane) => lane.commitId === parentId);
      const parentLane = lanes[parentLaneIndex];
      return {
        parentId,
        parentRowIndex: null,
        laneIndex: parentLaneIndex === -1 ? laneIndex : parentLaneIndex,
        colorIndex: parentLane?.colorIndex ?? currentLane.colorIndex,
      } satisfies RepositoryGraphParentLayout;
    });

    return {
      commit,
      laneIndex,
      laneColorIndex: currentLane.colorIndex,
      laneCount: Math.max(
        1,
        laneIndex + 1,
        lanes.length,
        ...parents.map((parent) => parent.laneIndex + 1),
      ),
      parents,
    } satisfies RepositoryGraphRowLayout;
  });

  const rowIndexByCommitId = new Map(rows.map((row, rowIndex) => [row.commit.id, rowIndex]));
  const laneCount = Math.max(1, ...rows.map((row) => row.laneCount));

  return {
    laneCount,
    rows: rows.map((row) => ({
      ...row,
      laneCount,
      parents: row.parents.map((parent) => ({
        ...parent,
        parentRowIndex: rowIndexByCommitId.get(parent.parentId) ?? null,
      })),
    })),
  };
}
