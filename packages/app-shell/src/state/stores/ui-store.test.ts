import { describe, it, expect, beforeEach } from "vitest";
import { useUiStore, UI_STORAGE_KEY } from "./ui-store";
import { flushDebouncedPersistStorage } from "./debounced-json-storage";
import type { Session } from "@ora/contracts";
import { AGENT_REF } from "../../test/agent-identity";

beforeEach(() => {
  flushDebouncedPersistStorage();
  window.localStorage.clear();
  useUiStore.setState({
    sidebarCollapsed: false,
    settingsOpen: false,
    settingsCategory: "appearance",
    workflowEditorOpen: false,
    expandedProjects: new Set<string>(),
    expandedTasks: new Set<string>(),
    treeExpansionBootstrapped: false,
    dialog: null,
    deleteTarget: null,
  });
  // setState enqueues a persist write; drain and clear so rehydrate tests seed
  // localStorage without losing to a pending default snapshot.
  flushDebouncedPersistStorage();
  window.localStorage.clear();
});

describe("useUiStore", () => {
  it("toggles sidebar collapse", () => {
    useUiStore.getState().setSidebarCollapsed(true);
    expect(useUiStore.getState().sidebarCollapsed).toBe(true);
    useUiStore.getState().setSidebarCollapsed(false);
    expect(useUiStore.getState().sidebarCollapsed).toBe(false);
  });

  it("toggles settings dialog open state", () => {
    useUiStore.getState().setSettingsOpen(true);
    expect(useUiStore.getState().settingsOpen).toBe(true);
  });
  it("opens settings on the requested category", () => {
    useUiStore.getState().openSettingsAt("plugins");
    expect(useUiStore.getState().settingsOpen).toBe(true);
    expect(useUiStore.getState().settingsCategory).toBe("plugins");
    useUiStore.getState().setSettingsCategory("appearance");
    expect(useUiStore.getState().settingsCategory).toBe("appearance");
  });
  it("toggles the workflow editor without persisting it", () => {
    useUiStore.getState().setWorkflowEditorOpen(true);
    expect(useUiStore.getState().workflowEditorOpen).toBe(true);
    useUiStore.getState().setWorkflowEditorOpen(false);
    expect(useUiStore.getState().workflowEditorOpen).toBe(false);
  });

  it("toggles project expansion and produces a new Set each time", () => {
    const initial = useUiStore.getState().expandedProjects;
    useUiStore.getState().toggleProjectExpand("p1");
    expect(useUiStore.getState().expandedProjects).toEqual(new Set(["p1"]));
    expect(useUiStore.getState().expandedProjects).not.toBe(initial);
    useUiStore.getState().toggleProjectExpand("p1");
    expect(useUiStore.getState().expandedProjects).toEqual(new Set());
  });

  it("toggles task expansion independently from projects", () => {
    useUiStore.getState().toggleTaskExpand("t1");
    expect(useUiStore.getState().expandedTasks).toEqual(new Set(["t1"]));
    expect(useUiStore.getState().expandedProjects).toEqual(new Set());
  });

  it("expandProject is idempotent and never collapses", () => {
    useUiStore.getState().expandProject("p1");
    useUiStore.getState().expandProject("p1");
    expect(useUiStore.getState().expandedProjects).toEqual(new Set(["p1"]));
  });

  it("expandTask is idempotent and never collapses", () => {
    useUiStore.getState().expandTask("t1");
    useUiStore.getState().expandTask("t1");
    expect(useUiStore.getState().expandedTasks).toEqual(new Set(["t1"]));
  });

  it("bootstrapTreeExpansion seeds once and then no-ops", () => {
    useUiStore.getState().bootstrapTreeExpansion(["p1"], ["t1"]);
    expect(useUiStore.getState().expandedProjects).toEqual(new Set(["p1"]));
    expect(useUiStore.getState().expandedTasks).toEqual(new Set(["t1"]));
    expect(useUiStore.getState().treeExpansionBootstrapped).toBe(true);

    useUiStore.getState().toggleProjectExpand("p1");
    useUiStore.getState().bootstrapTreeExpansion(["p1", "p2"], ["t1", "t2"]);
    expect(useUiStore.getState().expandedProjects).toEqual(new Set());
    expect(useUiStore.getState().expandedTasks).toEqual(new Set(["t1"]));
  });

  it("marks treeExpansionBootstrapped when the user toggles a row", () => {
    expect(useUiStore.getState().treeExpansionBootstrapped).toBe(false);
    useUiStore.getState().toggleProjectExpand("p1");
    expect(useUiStore.getState().treeExpansionBootstrapped).toBe(true);
    expect(useUiStore.getState().expandedProjects).toEqual(new Set(["p1"]));
  });

  it("pruneTreeExpansion drops ids that are no longer in the live tree", () => {
    useUiStore.setState({
      expandedProjects: new Set(["p1", "gone"]),
      expandedTasks: new Set(["t1", "gone-task"]),
      treeExpansionBootstrapped: true,
    });
    useUiStore.getState().pruneTreeExpansion(["p1"], ["t1"]);
    expect(useUiStore.getState().expandedProjects).toEqual(new Set(["p1"]));
    expect(useUiStore.getState().expandedTasks).toEqual(new Set(["t1"]));
  });

  it("round-trips a collapsed tree through localStorage without re-expanding", async () => {
    useUiStore.getState().bootstrapTreeExpansion(["p1", "p2"], ["t1"]);
    useUiStore.getState().toggleProjectExpand("p1");
    expect(useUiStore.getState().expandedProjects).toEqual(new Set(["p2"]));
    flushDebouncedPersistStorage();

    await useUiStore.persist.rehydrate();
    expect(useUiStore.getState().treeExpansionBootstrapped).toBe(true);
    expect(useUiStore.getState().expandedProjects).toEqual(new Set(["p2"]));
    expect(useUiStore.getState().expandedTasks).toEqual(new Set(["t1"]));

    useUiStore.getState().bootstrapTreeExpansion(["p1", "p2"], ["t1"]);
    expect(useUiStore.getState().expandedProjects).toEqual(new Set(["p2"]));
  });

  it("persists layout preferences to localStorage under the v1 key", () => {
    useUiStore.getState().setSidebarCollapsed(true);
    useUiStore.getState().toggleProjectExpand("p1");
    useUiStore.getState().toggleTaskExpand("t1");
    useUiStore.getState().bootstrapTreeExpansion([], []);
    flushDebouncedPersistStorage();

    const raw = window.localStorage.getItem(UI_STORAGE_KEY);
    expect(raw).not.toBeNull();
    const parsed = JSON.parse(raw!) as {
      state: {
        sidebarCollapsed: boolean;
        expandedProjects: string[];
        expandedTasks: string[];
        treeExpansionBootstrapped: boolean;
      };
    };
    expect(parsed.state).toEqual({
      sidebarCollapsed: true,
      expandedProjects: ["p1"],
      expandedTasks: ["t1"],
      treeExpansionBootstrapped: true,
    });
    expect(raw!).not.toContain("settingsOpen");
    expect(raw!).not.toContain("workflowEditorOpen");
    expect(raw!).not.toContain("dialog");
  });

  it("rehydrates layout preferences and rebuilds expand Sets", async () => {
    window.localStorage.setItem(
      UI_STORAGE_KEY,
      JSON.stringify({
        state: {
          sidebarCollapsed: true,
          expandedProjects: ["p1", "p2"],
          expandedTasks: ["t1"],
          treeExpansionBootstrapped: true,
        },
        version: 0,
      }),
    );
    await useUiStore.persist.rehydrate();
    expect(useUiStore.getState().sidebarCollapsed).toBe(true);
    expect(useUiStore.getState().expandedProjects).toEqual(
      new Set(["p1", "p2"]),
    );
    expect(useUiStore.getState().expandedTasks).toEqual(new Set(["t1"]));
    expect(useUiStore.getState().treeExpansionBootstrapped).toBe(true);
  });

  it("keeps pre-hydration expand toggles when async rehydrate completes", async () => {
    window.localStorage.setItem(
      UI_STORAGE_KEY,
      JSON.stringify({
        state: {
          sidebarCollapsed: false,
          expandedProjects: ["p1"],
          expandedTasks: [],
          treeExpansionBootstrapped: true,
        },
        version: 0,
      }),
    );
    useUiStore.setState({
      sidebarCollapsed: false,
      expandedProjects: new Set(["p1"]),
      expandedTasks: new Set(),
      treeExpansionBootstrapped: true,
      settingsOpen: false,
      workflowEditorOpen: false,
      dialog: null,
      deleteTarget: null,
    });
    useUiStore.getState().toggleProjectExpand("p1");
    expect(useUiStore.getState().expandedProjects).toEqual(new Set());

    await useUiStore.persist.rehydrate();

    expect(useUiStore.getState().expandedProjects).toEqual(new Set());
  });

  it("drops corrupt non-string expand ids", async () => {
    window.localStorage.setItem(
      UI_STORAGE_KEY,
      JSON.stringify({
        state: {
          sidebarCollapsed: "yes",
          expandedProjects: ["p1", 42, null, ""],
          expandedTasks: "t1",
          treeExpansionBootstrapped: 1,
        },
        version: 0,
      }),
    );
    await useUiStore.persist.rehydrate();
    expect(useUiStore.getState().sidebarCollapsed).toBe(false);
    expect(useUiStore.getState().expandedProjects).toEqual(new Set(["p1"]));
    expect(useUiStore.getState().expandedTasks).toEqual(new Set());
    expect(useUiStore.getState().treeExpansionBootstrapped).toBe(false);
  });

  it("falls back to defaults when persisted JSON is corrupt", async () => {
    window.localStorage.setItem(UI_STORAGE_KEY, "{not json");
    await useUiStore.persist.rehydrate();
    expect(useUiStore.persist.hasHydrated()).toBe(true);
    expect(useUiStore.getState().sidebarCollapsed).toBe(false);
    expect(useUiStore.getState().expandedProjects).toEqual(new Set());
    expect(useUiStore.getState().expandedTasks).toEqual(new Set());
    expect(useUiStore.getState().treeExpansionBootstrapped).toBe(false);
  });

  it("stores the active dialog and delete target verbatim", () => {
    const session: Session = {
      id: "s1",
      workspaceId: "workspace-t1",
      agentRef: AGENT_REF.opencode,
      status: "running",
      title: null,
      historyState: { type: "writable" },
    };

    useUiStore.getState().setDialog({ kind: "project" });
    useUiStore.getState().setDeleteTarget({
      kind: "project",
      id: "p1",
      name: "Ora",
      sessionIds: ["s1"],
    });

    expect(useUiStore.getState().dialog).toEqual({
      kind: "project",
    });
    expect(useUiStore.getState().deleteTarget).toEqual({
      kind: "project",
      id: "p1",
      name: "Ora",
      sessionIds: ["s1"],
    });

    // Dialog state with task/session kinds preserves their lineage fields.
    useUiStore.getState().setDialog({ kind: "task", projectId: "p1" });
    expect(useUiStore.getState().dialog).toEqual({
      kind: "task",
      projectId: "p1",
    });
    useUiStore.getState().setDialog({
      kind: "session",
      workspaceId: "workspace-t1",
      entity: session,
    });
    expect(useUiStore.getState().dialog).toEqual({
      kind: "session",
      workspaceId: "workspace-t1",
      entity: session,
    });

    useUiStore.getState().setDialog(null);
    useUiStore.getState().setDeleteTarget(null);
    expect(useUiStore.getState().dialog).toBeNull();
    expect(useUiStore.getState().deleteTarget).toBeNull();
  });
});
