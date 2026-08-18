import { describe, it, expect, beforeEach } from "vitest";
import { useUiStore } from "./ui-store";
import type { Session } from "@ora/contracts";

beforeEach(() => {
  useUiStore.setState({
    sidebarCollapsed: false,
    settingsOpen: false,
    expandedProjects: new Set<string>(),
    expandedTasks: new Set<string>(),
    dialog: null,
    deleteTarget: null,
  });
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

  it("stores the active dialog and delete target verbatim", () => {
    const session: Session = {
      id: "s1",
      taskId: "t1",
      agentCli: "open_code",
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
    useUiStore
      .getState()
      .setDialog({ kind: "session", taskId: "t1", entity: session });
    expect(useUiStore.getState().dialog).toEqual({
      kind: "session",
      taskId: "t1",
      entity: session,
    });

    useUiStore.getState().setDialog(null);
    useUiStore.getState().setDeleteTarget(null);
    expect(useUiStore.getState().dialog).toBeNull();
    expect(useUiStore.getState().deleteTarget).toBeNull();
  });
});
