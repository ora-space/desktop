import { describe, it, expect, beforeEach } from "vitest";
import {
  buildWorkflowReminder,
  getRun,
  kickNode,
  suggestedNextNode,
  useWorkflowStore,
  workflowKeyFor,
} from "./workflow-store";

const KEY = "session-1";

beforeEach(() => {
  useWorkflowStore.setState({ runs: {} });
});

function run(key = KEY) {
  return getRun(useWorkflowStore.getState(), key);
}

describe("useWorkflowStore", () => {
  it("reports an inactive, hidden, all-pending run for an unknown session", () => {
    const state = run();
    expect(state.active).toBe(false);
    expect(state.visible).toBe(false);
    expect(state.currentNodeId).toBeNull();
    expect(state.nodes.map((node) => node.status)).toEqual([
      "pending",
      "pending",
      "pending",
      "pending",
      "pending",
    ]);
  });

  it("first toggle starts a visible run", () => {
    useWorkflowStore.getState().toggleVisible(KEY);
    expect(run().active).toBe(true);
    expect(run().visible).toBe(true);
  });

  it("toggling again only hides the run, preserving its progress", () => {
    useWorkflowStore.getState().toggleVisible(KEY);
    useWorkflowStore.getState().launchNode(KEY, "propose");
    useWorkflowStore.getState().toggleVisible(KEY);
    expect(run().visible).toBe(false);
    expect(run().active).toBe(true);
    expect(run().currentNodeId).toBe("propose");
    // Showing again keeps the same progress rather than starting over.
    useWorkflowStore.getState().toggleVisible(KEY);
    expect(run().visible).toBe(true);
    expect(run().currentNodeId).toBe("propose");
  });

  it("reset cancels the run entirely", () => {
    useWorkflowStore.getState().toggleVisible(KEY);
    useWorkflowStore.getState().reset(KEY);
    expect(useWorkflowStore.getState().runs[KEY]).toBeUndefined();
    expect(run().active).toBe(false);
  });

  it("keeps runs isolated per session", () => {
    useWorkflowStore.getState().toggleVisible(KEY);
    useWorkflowStore.getState().launchNode(KEY, "propose");
    expect(run("session-2").active).toBe(false);
    expect(run("session-2").currentNodeId).toBeNull();
    expect(run(KEY).currentNodeId).toBe("propose");
  });

  it("completing a node clears it as current and marks it done", () => {
    useWorkflowStore.getState().toggleVisible(KEY);
    useWorkflowStore.getState().launchNode(KEY, "propose");
    useWorkflowStore.getState().completeNode(KEY, "propose");
    expect(run().currentNodeId).toBeNull();
    expect(run().nodes.find((node) => node.id === "propose")?.status).toBe("done");
  });

  it("skipping marks the node skipped", () => {
    useWorkflowStore.getState().toggleVisible(KEY);
    useWorkflowStore.getState().skipNode(KEY, "sync");
    expect(run().nodes.find((node) => node.id === "sync")?.status).toBe("skipped");
  });

  it("rekey moves a run to a new key", () => {
    useWorkflowStore.getState().toggleVisible(KEY);
    useWorkflowStore.getState().rekey(KEY, "real-session");
    expect(useWorkflowStore.getState().runs[KEY]).toBeUndefined();
    expect(run("real-session").active).toBe(true);
  });
});

describe("kickNode", () => {
  it("is null for an inactive run", () => {
    expect(kickNode(run())).toBeNull();
  });

  it("starts at explore for a fresh visible run", () => {
    useWorkflowStore.getState().toggleVisible(KEY);
    expect(kickNode(run())).toBe("explore");
  });

  it("moves to propose after explore is skipped", () => {
    useWorkflowStore.getState().toggleVisible(KEY);
    useWorkflowStore.getState().skipNode(KEY, "explore");
    // Regression: skipping the highlighted stage must re-point the kick target,
    // so a typed message launches propose rather than the skipped explore.
    expect(kickNode(run())).toBe("propose");
  });

  it("is null while a stage is running (within-stage messages are plain chat)", () => {
    useWorkflowStore.getState().toggleVisible(KEY);
    useWorkflowStore.getState().launchNode(KEY, "explore");
    expect(kickNode(run())).toBeNull();
  });

  it("is null when the run is hidden", () => {
    useWorkflowStore.getState().toggleVisible(KEY);
    useWorkflowStore.getState().toggleVisible(KEY);
    expect(kickNode(run())).toBeNull();
  });
});

describe("buildWorkflowReminder", () => {
  it("embeds the absolute skills path and the node's skill name", () => {
    const text = buildWorkflowReminder("propose", "/repo/.opencode/skills");
    expect(text).toContain("/repo/.opencode/skills");
    expect(text).toContain("openspec-propose");
  });

  it("maps apply to the apply-change skill", () => {
    expect(buildWorkflowReminder("apply", "/x/.opencode/skills")).toContain("openspec-apply-change");
  });
});

describe("workflowKeyFor", () => {
  it("prefers the session id, then a task key, then a sentinel", () => {
    expect(workflowKeyFor({ sessionId: "s1", taskId: "t1" })).toBe("s1");
    expect(workflowKeyFor({ sessionId: null, taskId: "t1" })).toBe("task:t1");
    expect(workflowKeyFor({ sessionId: null, taskId: null })).toBe("__none__");
  });
});

describe("suggestedNextNode", () => {
  it("starts at explore", () => {
    expect(suggestedNextNode(run().nodes)).toBe("explore");
  });

  it("advances to the next pending node as nodes complete", () => {
    useWorkflowStore.getState().toggleVisible(KEY);
    useWorkflowStore.getState().completeNode(KEY, "explore");
    expect(suggestedNextNode(run().nodes)).toBe("propose");
  });

  it("returns null when nothing is pending", () => {
    const nodes = run().nodes.map((node) => ({ ...node, status: "done" as const }));
    expect(suggestedNextNode(nodes)).toBeNull();
  });
});
