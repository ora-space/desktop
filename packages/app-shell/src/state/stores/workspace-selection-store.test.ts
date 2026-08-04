import { describe, it, expect, beforeEach } from "vitest";
import { useWorkspaceSelectionStore } from "./workspace-selection-store";

beforeEach(() => {
  useWorkspaceSelectionStore.getState().clearSelection();
});

describe("useWorkspaceSelectionStore", () => {
  it("starts empty", () => {
    expect(useWorkspaceSelectionStore.getState().selection).toEqual({
      projectId: null,
      taskId: null,
      sessionId: null,
    });
  });

  it("selectProject sets project and clears task/session", () => {
    useWorkspaceSelectionStore.getState().selectSession("s1", "t1", "p1");
    useWorkspaceSelectionStore.getState().selectProject("p2");
    expect(useWorkspaceSelectionStore.getState().selection).toEqual({
      projectId: "p2",
      taskId: null,
      sessionId: null,
    });
  });

  it("selectTask records the owning project and clears session", () => {
    useWorkspaceSelectionStore.getState().selectSession("s1", "t1", "p1");
    useWorkspaceSelectionStore.getState().selectTask("t2", "p1");
    expect(useWorkspaceSelectionStore.getState().selection).toEqual({
      projectId: "p1",
      taskId: "t2",
      sessionId: null,
    });
  });

  it("selectSession records project, task, and session together", () => {
    useWorkspaceSelectionStore.getState().selectSession("s1", "t1", "p1");
    expect(useWorkspaceSelectionStore.getState().selection).toEqual({
      projectId: "p1",
      taskId: "t1",
      sessionId: "s1",
    });
  });

  it("clearSelection empties all three legs", () => {
    useWorkspaceSelectionStore.getState().selectSession("s1", "t1", "p1");
    useWorkspaceSelectionStore.getState().clearSelection();
    expect(useWorkspaceSelectionStore.getState().selection).toEqual({
      projectId: null,
      taskId: null,
      sessionId: null,
    });
  });

  it("clearSessionSelection keeps project and task", () => {
    useWorkspaceSelectionStore.getState().selectSession("s1", "t1", "p1");
    useWorkspaceSelectionStore.getState().clearSessionSelection();
    expect(useWorkspaceSelectionStore.getState().selection).toEqual({
      projectId: "p1",
      taskId: "t1",
      sessionId: null,
    });
  });

  it("clearTaskSelection keeps the project leg only", () => {
    useWorkspaceSelectionStore.getState().selectSession("s1", "t1", "p1");
    useWorkspaceSelectionStore.getState().clearTaskSelection("p1");
    expect(useWorkspaceSelectionStore.getState().selection).toEqual({
      projectId: "p1",
      taskId: null,
      sessionId: null,
    });
  });

  it("setProject(null) empties the whole selection", () => {
    useWorkspaceSelectionStore.getState().selectSession("s1", "t1", "p1");
    useWorkspaceSelectionStore.getState().setProject(null);
    expect(useWorkspaceSelectionStore.getState().selection).toEqual({
      projectId: null,
      taskId: null,
      sessionId: null,
    });
  });

  it("setProject(id) switches project and clears children", () => {
    useWorkspaceSelectionStore.getState().selectSession("s1", "t1", "p1");
    useWorkspaceSelectionStore.getState().setProject("p2");
    expect(useWorkspaceSelectionStore.getState().selection).toEqual({
      projectId: "p2",
      taskId: null,
      sessionId: null,
    });
  });
});
