import { beforeEach, describe, expect, it } from "vitest";
import {
  dismissSessionDraft,
  recoverFailedDraftSend,
  reparkDraftComposerContent,
  resetComposerSendAdoptionsForTests,
  resolveNewChatScope,
  selectBoundDraftSession,
  startSessionDraft,
} from "./session-drafts";
import { useComposerInputStore } from "./stores/composer-input-store";
import { useComposerPluginSelectionStore } from "./stores/composer-plugin-selection-store";
import { useDraftSessionsStore } from "./stores/draft-sessions-store";
import { useUiStore } from "./stores/ui-store";
import { useWorkflowStore } from "./stores/workflow-store";
import { useWorkspaceSelectionStore } from "./stores/workspace-selection-store";

beforeEach(() => {
  resetComposerSendAdoptionsForTests();
  useDraftSessionsStore.getState().clear();
  useComposerInputStore.getState().reset();
  useComposerPluginSelectionStore.setState({ selectedIdsByConversation: {} });
  useWorkflowStore.setState({ runs: {} });
  useWorkspaceSelectionStore.getState().clearSelection();
  useUiStore.setState({
    expandedProjects: new Set(),
    expandedTasks: new Set(),
    treeExpansionBootstrapped: false,
  });
});

describe("resolveNewChatScope", () => {
  const emptySelection = {
    projectId: null,
    taskId: null,
    sessionId: null,
    workflowRunId: null,
    draftId: null,
  };

  it("prefers createFocus over live selection", () => {
    expect(
      resolveNewChatScope(
        { projectId: "p2", taskId: null },
        {
          ...emptySelection,
          projectId: "p1",
          taskId: "t1",
          sessionId: "s1",
        },
        "p-first",
      ),
    ).toEqual({ projectId: "p2", taskId: null });
  });

  it("uses the selection task when createFocus is absent", () => {
    expect(
      resolveNewChatScope(
        null,
        {
          ...emptySelection,
          projectId: "p1",
          taskId: "t1",
          sessionId: "s1",
        },
        "p-first",
      ),
    ).toEqual({ projectId: "p1", taskId: "t1" });
  });

  it("falls back to the first project, then null", () => {
    expect(resolveNewChatScope(null, emptySelection, "p-first")).toEqual({
      projectId: "p-first",
      taskId: null,
    });
    expect(resolveNewChatScope(null, emptySelection, null)).toBeNull();
  });

  it("ignores createFocus whose project is missing from the tree", () => {
    expect(
      resolveNewChatScope(
        { projectId: "gone", taskId: null },
        emptySelection,
        "p-first",
        { projects: [{ id: "p1" }], tasks: [] },
      ),
    ).toEqual({ projectId: "p-first", taskId: null });
  });

  it("demotes createFocus to a project-level draft when its task is missing", () => {
    expect(
      resolveNewChatScope(
        { projectId: "p1", taskId: "gone" },
        emptySelection,
        "p-first",
        {
          projects: [{ id: "p1" }],
          tasks: [{ id: "t1", projectId: "p1" }],
        },
      ),
    ).toEqual({ projectId: "p1", taskId: null });
  });

  it("keeps a createFocus whose project and worktree task both still exist", () => {
    expect(
      resolveNewChatScope(
        { projectId: "p1", taskId: "t1" },
        emptySelection,
        "p-first",
        {
          projects: [{ id: "p1" }],
          tasks: [{ id: "t1", projectId: "p1" }],
        },
      ),
    ).toEqual({ projectId: "p1", taskId: "t1" });
  });

  it("keeps the selection's worktree task when createFocus is absent and the task is in the tree", () => {
    expect(
      resolveNewChatScope(
        null,
        { ...emptySelection, projectId: "p1", taskId: "t1" },
        "p-first",
        {
          projects: [{ id: "p1" }],
          tasks: [{ id: "t1", projectId: "p1" }],
        },
      ),
    ).toEqual({ projectId: "p1", taskId: "t1" });
  });

  it("falls back to the first project when the selection's project is gone from the tree", () => {
    expect(
      resolveNewChatScope(
        null,
        {
          ...emptySelection,
          projectId: "gone",
          taskId: "t1",
          sessionId: "s1",
        },
        "p-first",
        {
          projects: [{ id: "p1" }],
          tasks: [{ id: "t1", projectId: "p1" }],
        },
      ),
    ).toEqual({ projectId: "p-first", taskId: null });
  });
});

describe("startSessionDraft", () => {
  it("selects a new empty draft and expands its ancestors", () => {
    const id = startSessionDraft({ projectId: "p1", taskId: "t1" });
    expect(useWorkspaceSelectionStore.getState().selection).toEqual({
      projectId: "p1",
      taskId: "t1",
      sessionId: null,
      workflowRunId: null,
      draftId: id,
    });
    expect(useUiStore.getState().expandedProjects.has("p1")).toBe(true);
    expect(useUiStore.getState().expandedTasks.has("t1")).toBe(true);
  });

  it("keeps returnTo when New is clicked again on the same empty draft", () => {
    useWorkspaceSelectionStore.getState().selectSession("s1", "t1", "p1");
    const id = startSessionDraft({ projectId: "p1", taskId: "t1" });
    expect(
      useDraftSessionsStore.getState().drafts.find((d) => d.id === id)
        ?.returnTo,
    ).toEqual({
      sessionId: "s1",
      taskId: "t1",
      projectId: "p1",
    });

    const again = startSessionDraft({ projectId: "p1", taskId: "t1" });
    expect(again).toBe(id);
    expect(
      useDraftSessionsStore.getState().drafts.find((d) => d.id === id)
        ?.returnTo,
    ).toEqual({
      sessionId: "s1",
      taskId: "t1",
      projectId: "p1",
    });
  });

  it("inherits returnTo when chaining New from a typed draft", () => {
    useWorkspaceSelectionStore.getState().selectSession("s1", "t1", "p1");
    const first = startSessionDraft({ projectId: "p1", taskId: "t1" });
    useDraftSessionsStore.getState().updateContent(first, { text: "parked" });
    const second = startSessionDraft({ projectId: "p1", taskId: "t1" });
    expect(second).not.toBe(first);
    expect(
      useDraftSessionsStore.getState().drafts.find((d) => d.id === second)
        ?.returnTo,
    ).toEqual({
      sessionId: "s1",
      taskId: "t1",
      projectId: "p1",
    });
  });
});

describe("selectBoundDraftSession", () => {
  it("selects the pending session under its worktree", () => {
    selectBoundDraftSession({
      projectId: "p1",
      taskId: "t1",
      pendingSessionId: "s1",
    });
    expect(useWorkspaceSelectionStore.getState().selection).toEqual({
      projectId: "p1",
      taskId: "t1",
      sessionId: "s1",
      workflowRunId: null,
      draftId: null,
    });
  });

  it("selects a direct-chat pending session before its task exists", () => {
    selectBoundDraftSession({
      projectId: "p1",
      taskId: null,
      pendingSessionId: "s1",
    });
    expect(useWorkspaceSelectionStore.getState().selection).toEqual({
      projectId: "p1",
      taskId: null,
      sessionId: "s1",
      workflowRunId: null,
      draftId: null,
    });
  });
});

describe("recoverFailedDraftSend", () => {
  it("restores decoded image sizes so attachment limits remain accurate", () => {
    const id = startSessionDraft({ projectId: "p1", taskId: null });

    reparkDraftComposerContent({
      draftId: id,
      text: "retry",
      images: [{ mimeType: "image/png", data: "YWJjZA==" }],
    });

    expect(
      useDraftSessionsStore.getState().drafts.find((draft) => draft.id === id)
        ?.images,
    ).toEqual([
      expect.objectContaining({
        name: "image-1",
        size: 4,
      }),
    ]);
  });

  it("parks TipTap doc alongside text so chips survive abandon repark", () => {
    const id = startSessionDraft({ projectId: "p1", taskId: null });
    const doc = {
      type: "doc",
      content: [
        {
          type: "paragraph",
          content: [
            {
              type: "promptToken",
              attrs: { kind: "skill", name: "code-review" },
            },
          ],
        },
      ],
    };

    reparkDraftComposerContent({
      draftId: id,
      text: "$code-review ",
      doc,
    });

    expect(useComposerInputStore.getState().byKey[`draft:${id}`]).toEqual({
      text: "$code-review ",
      images: [],
      doc,
    });
  });

  it("unbinds the dead warm id, reselects the draft, and re-parks the message", () => {
    const id = startSessionDraft({ projectId: "p1", taskId: null });
    useDraftSessionsStore.getState().updateContent(id, { text: "hello" });
    useDraftSessionsStore.getState().bindToSession(id, "warm-dead");
    useWorkspaceSelectionStore
      .getState()
      .selectSessionBeforeTask("warm-dead", "p1");
    useComposerInputStore.getState().setInput("warm-dead", {
      text: "stale",
      images: [],
    });

    recoverFailedDraftSend({
      draftId: id,
      projectId: "p1",
      taskId: "t-created",
      text: "hello",
      boundSessionId: "warm-dead",
    });

    const draft = useDraftSessionsStore
      .getState()
      .drafts.find((candidate) => candidate.id === id);
    expect(draft).toEqual(
      expect.objectContaining({
        id,
        pendingSessionId: null,
        taskId: "t-created",
        text: "hello",
      }),
    );
    expect(useWorkspaceSelectionStore.getState().selection).toEqual({
      projectId: "p1",
      taskId: "t-created",
      sessionId: null,
      workflowRunId: null,
      draftId: id,
    });
    expect(useComposerInputStore.getState().byKey[`draft:${id}`]?.text).toBe(
      "hello",
    );
    expect(useComposerInputStore.getState().byKey["warm-dead"]).toBeUndefined();
  });

  it("moves plugin picks and workflow state back onto the draft key", () => {
    const id = startSessionDraft({ projectId: "p1", taskId: null });
    useComposerPluginSelectionStore
      .getState()
      .addPlugin("warm-dead", "plugin-a");
    useWorkflowStore.getState().toggleVisible("warm-dead");
    useWorkspaceSelectionStore
      .getState()
      .selectSessionBeforeTask("warm-dead", "p1");
    useDraftSessionsStore.getState().bindToSession(id, "warm-dead");

    recoverFailedDraftSend({
      draftId: id,
      projectId: "p1",
      taskId: null,
      text: "retry me",
      boundSessionId: "warm-dead",
    });

    const draftKey = `draft:${id}`;
    expect(
      useComposerPluginSelectionStore.getState().selectedIdsByConversation[
        draftKey
      ],
    ).toEqual(["plugin-a"]);
    expect(
      useComposerPluginSelectionStore.getState().selectedIdsByConversation[
        "warm-dead"
      ],
    ).toBeUndefined();
    expect(useWorkflowStore.getState().runs[draftKey]?.visible).toBe(true);
    expect(useWorkflowStore.getState().runs["warm-dead"]).toBeUndefined();
  });

  it("rekeys from the bound warm id even when selection already moved elsewhere", () => {
    const id = startSessionDraft({ projectId: "p1", taskId: null });
    useComposerPluginSelectionStore
      .getState()
      .addPlugin("warm-bound", "plugin-a");
    useWorkflowStore.getState().toggleVisible("warm-bound");
    useDraftSessionsStore.getState().bindToSession(id, "warm-bound");
    // User left the in-flight chat for another session before attach failed.
    useWorkspaceSelectionStore.getState().selectSession("other-s", "t1", "p1");
    useComposerPluginSelectionStore
      .getState()
      .addPlugin("other-s", "plugin-other");

    recoverFailedDraftSend({
      draftId: id,
      projectId: "p1",
      taskId: null,
      text: "retry me",
      boundSessionId: "warm-bound",
    });

    const draftKey = `draft:${id}`;
    expect(
      useComposerPluginSelectionStore.getState().selectedIdsByConversation[
        draftKey
      ],
    ).toEqual(["plugin-a"]);
    expect(
      useComposerPluginSelectionStore.getState().selectedIdsByConversation[
        "warm-bound"
      ],
    ).toBeUndefined();
    expect(
      useComposerPluginSelectionStore.getState().selectedIdsByConversation[
        "other-s"
      ],
    ).toEqual(["plugin-other"]);
    expect(useWorkflowStore.getState().runs[draftKey]?.visible).toBe(true);
    expect(useWorkflowStore.getState().runs["warm-bound"]).toBeUndefined();
  });
});

describe("dismissSessionDraft", () => {
  it("falls back to a sibling draft in the same scope", () => {
    const first = startSessionDraft({ projectId: "p1", taskId: null });
    useDraftSessionsStore.getState().updateContent(first, { text: "parked" });
    const second = startSessionDraft({ projectId: "p1", taskId: null });
    dismissSessionDraft(second);
    expect(useWorkspaceSelectionStore.getState().selection.draftId).toBe(first);
    expect(
      useDraftSessionsStore.getState().drafts.map((draft) => draft.id),
    ).toEqual([first]);
  });

  it("uses the draft id as a deterministic tie-breaker for siblings", () => {
    const first = startSessionDraft({ projectId: "p1", taskId: null });
    useDraftSessionsStore.getState().updateContent(first, { text: "first" });
    const second = startSessionDraft({ projectId: "p1", taskId: null });
    useDraftSessionsStore.getState().updateContent(second, { text: "second" });
    const dismissed = startSessionDraft({ projectId: "p1", taskId: null });
    useDraftSessionsStore.setState((state) => ({
      drafts: state.drafts.map((draft) => ({ ...draft, updatedAt: 1 })),
    }));

    dismissSessionDraft(dismissed);

    expect(useWorkspaceSelectionStore.getState().selection.draftId).toBe(
      [first, second].sort((left, right) => left.localeCompare(right))[0],
    );
  });

  it("falls back to the parent project when dismissing the last draft", () => {
    const id = startSessionDraft({ projectId: "p1", taskId: null });
    dismissSessionDraft(id);
    expect(useWorkspaceSelectionStore.getState().selection).toEqual({
      projectId: "p1",
      taskId: null,
      sessionId: null,
      workflowRunId: null,
      draftId: null,
    });
    expect(useDraftSessionsStore.getState().drafts).toHaveLength(0);
  });

  it("returns to the session the user left when dismissing an unused draft", () => {
    useWorkspaceSelectionStore.getState().selectSession("s1", "t1", "p1");
    const id = startSessionDraft({ projectId: "p1", taskId: "t1" });
    expect(
      useDraftSessionsStore.getState().drafts.find((d) => d.id === id)
        ?.returnTo,
    ).toEqual({
      sessionId: "s1",
      taskId: "t1",
      projectId: "p1",
    });

    dismissSessionDraft(id);

    expect(useDraftSessionsStore.getState().drafts).toHaveLength(0);
    expect(useWorkspaceSelectionStore.getState().selection).toEqual({
      projectId: "p1",
      taskId: "t1",
      sessionId: "s1",
      workflowRunId: null,
      draftId: null,
    });
    expect(useUiStore.getState().expandedProjects.has("p1")).toBe(true);
    expect(useUiStore.getState().expandedTasks.has("t1")).toBe(true);
  });

  it("prefers a sibling draft over returnTo when dismissing", () => {
    useWorkspaceSelectionStore.getState().selectSession("s1", "t1", "p1");
    const first = startSessionDraft({ projectId: "p1", taskId: null });
    useDraftSessionsStore.getState().updateContent(first, { text: "parked" });
    useWorkspaceSelectionStore.getState().selectSession("s2", "t1", "p1");
    const second = startSessionDraft({ projectId: "p1", taskId: null });
    dismissSessionDraft(second);
    expect(useWorkspaceSelectionStore.getState().selection.draftId).toBe(first);
  });

  it("removes a bound draft without leaving the live session", () => {
    const id = startSessionDraft({ projectId: "p1", taskId: "t1" });
    useDraftSessionsStore.getState().updateContent(id, { text: "sending" });
    useDraftSessionsStore.getState().bindToSession(id, "s1");
    useWorkspaceSelectionStore.getState().selectSession("s1", "t1", "p1");

    dismissSessionDraft(id);

    expect(useDraftSessionsStore.getState().drafts).toHaveLength(0);
    expect(useWorkspaceSelectionStore.getState().selection).toEqual({
      projectId: "p1",
      taskId: "t1",
      sessionId: "s1",
      workflowRunId: null,
      draftId: null,
    });
  });

  it("refuses to dismiss a draft whose first send is still in flight", () => {
    const id = startSessionDraft({ projectId: "p1", taskId: null });
    useDraftSessionsStore.getState().updateContent(id, { text: "sending" });
    useDraftSessionsStore.getState().beginSend(id);

    dismissSessionDraft(id);

    expect(useDraftSessionsStore.getState().drafts.map((d) => d.id)).toEqual([
      id,
    ]);
    expect(useWorkspaceSelectionStore.getState().selection.draftId).toBe(id);
  });
});
