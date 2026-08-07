import type { RepositoryCommit } from "@ora/contracts";
import { describe, expect, it } from "vitest";
import { buildRepositoryGraphLayout, getRepositoryAuthorInitials } from "./repository-graph-layout";

function commit(id: string, parents: string[]): RepositoryCommit {
  return {
    id,
    shortId: id,
    parents,
    subject: id,
    authorName: "Ora",
    authorEmail: "ora@example.com",
    authoredAt: "2026-08-05T00:00:00Z",
    referenceNames: [],
  };
}

describe("buildRepositoryGraphLayout", () => {
  it("keeps a linear history in one lane", () => {
    const layout = buildRepositoryGraphLayout([
      commit("c3", ["c2"]),
      commit("c2", ["c1"]),
      commit("c1", []),
    ]);

    expect(layout.rows.map((row) => row.laneIndex)).toEqual([0, 0, 0]);
    expect(layout.rows[0].parents[0].parentRowIndex).toBe(1);
    expect(layout.laneCount).toBe(1);
  });

  it("keeps both merge parents connected to their actual rows", () => {
    const layout = buildRepositoryGraphLayout([
      commit("merge", ["main", "feature"]),
      commit("main", ["root"]),
      commit("feature", ["root"]),
      commit("root", []),
    ]);

    expect(layout.rows[0].parents.map((parent) => parent.parentRowIndex)).toEqual([1, 2]);
    expect(layout.rows[0].parents.map((parent) => parent.laneIndex)).toEqual([0, 1]);
    expect(layout.rows[1].laneIndex).toBe(0);
    expect(layout.rows[2].laneIndex).toBe(1);
    expect(layout.rows[2].parents[0].parentRowIndex).toBe(3);
  });

  it("ends an edge at the graph boundary when history is truncated", () => {
    const layout = buildRepositoryGraphLayout([commit("head", ["older-not-loaded"]) ]);

    expect(layout.rows[0].parents[0].parentRowIndex).toBeNull();
    expect(layout.rows[0].parents[0].laneIndex).toBe(0);
  });
});

describe("getRepositoryAuthorInitials", () => {
  it("uses the first and last name for a readable commit marker", () => {
    expect(getRepositoryAuthorInitials("Jane Doe", "jane@example.com")).toBe("JD");
  });

  it("falls back to the author email when Git does not provide a name", () => {
    expect(getRepositoryAuthorInitials("", "ora@example.com")).toBe("OR");
  });
});
