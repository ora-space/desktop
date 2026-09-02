import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { useComposerInputStore } from "./composer-input-store";
import { flushDebouncedPersistStorage } from "./debounced-json-storage";
import {
  draftHasContent,
  draftPlacements,
  draftPlacementsEqual,
  draftSidebarTitle,
  sameDraftScope,
  SESSION_DRAFTS_STORAGE_KEY,
  useDraftSessionsStore,
} from "./draft-sessions-store";

beforeEach(() => {
  flushDebouncedPersistStorage();
  window.localStorage.clear();
  useDraftSessionsStore.getState().clear();
  useComposerInputStore.getState().reset();
  flushDebouncedPersistStorage();
  window.localStorage.clear();
});

afterEach(() => {
  useDraftSessionsStore.getState().clear();
  useComposerInputStore.getState().reset();
  flushDebouncedPersistStorage();
  window.localStorage.clear();
});

describe("draftSidebarTitle", () => {
  it("uses the first non-empty line and falls back when the composer is blank", () => {
    expect(draftSidebarTitle("  hello world  \nsecond", "New session")).toBe(
      "hello world",
    );
    expect(draftSidebarTitle("   ", "New session")).toBe("New session");
  });
});

describe("draftPlacementsEqual", () => {
  it("ignores composer text when comparing tree structure", () => {
    const id = useDraftSessionsStore
      .getState()
      .ensureEmptyDraft({ projectId: "p1", taskId: null });
    const before = draftPlacements(useDraftSessionsStore.getState().drafts);
    useDraftSessionsStore.getState().updateContent(id, { text: "typed" });
    const after = draftPlacements(useDraftSessionsStore.getState().drafts);
    expect(draftPlacementsEqual(before, after)).toBe(true);
  });
});

describe("useDraftSessionsStore", () => {
  it("reuses one empty draft per scope and leaves a typed draft in place", () => {
    const first = useDraftSessionsStore
      .getState()
      .ensureEmptyDraft({ projectId: "p1", taskId: null });
    const again = useDraftSessionsStore
      .getState()
      .ensureEmptyDraft({ projectId: "p1", taskId: null });
    expect(again).toBe(first);

    useDraftSessionsStore.getState().updateContent(first, { text: "hello" });
    const next = useDraftSessionsStore
      .getState()
      .ensureEmptyDraft({ projectId: "p1", taskId: null });
    expect(next).not.toBe(first);
    expect(useDraftSessionsStore.getState().drafts).toHaveLength(2);
  });

  it("keeps a typed draft when leaving and drops an empty one", () => {
    const typed = useDraftSessionsStore
      .getState()
      .ensureEmptyDraft({ projectId: "p1", taskId: "t1" });
    useDraftSessionsStore.getState().updateContent(typed, { text: "keep me" });
    useDraftSessionsStore.getState().discardIfEmpty(typed);
    expect(
      useDraftSessionsStore.getState().drafts.map((draft) => draft.id),
    ).toEqual([typed]);

    const empty = useDraftSessionsStore
      .getState()
      .ensureEmptyDraft({ projectId: "p1", taskId: "t1" });
    useDraftSessionsStore.getState().discardIfEmpty(empty);
    expect(
      useDraftSessionsStore.getState().drafts.map((draft) => draft.id),
    ).toEqual([typed]);
  });

  it("drops a bound draft once its session is persisted", () => {
    const id = useDraftSessionsStore
      .getState()
      .ensureEmptyDraft({ projectId: "p1", taskId: null });
    useDraftSessionsStore.getState().updateContent(id, { text: "go" });
    useDraftSessionsStore.getState().bindToSession(id, "s1");
    useDraftSessionsStore.getState().discardIfEmpty(id);
    expect(useDraftSessionsStore.getState().drafts).toHaveLength(1);

    useDraftSessionsStore.getState().removeCommitted(["s1"]);
    expect(useDraftSessionsStore.getState().drafts).toHaveLength(0);
  });

  it("does not notify subscribers when content is unchanged", () => {
    const id = useDraftSessionsStore
      .getState()
      .ensureEmptyDraft({ projectId: "p1", taskId: null });
    useDraftSessionsStore.getState().updateContent(id, { text: "same" });
    const before = useDraftSessionsStore.getState().drafts;
    useDraftSessionsStore.getState().updateContent(id, { text: "same" });
    expect(useDraftSessionsStore.getState().drafts).toBe(before);

    useDraftSessionsStore.getState().removeCommitted(["other"]);
    expect(useDraftSessionsStore.getState().drafts).toBe(before);
  });

  it("treats images as content even without text", () => {
    const draft = {
      id: "d1",
      projectId: "p1",
      taskId: null,
      text: "  ",
      images: [
        {
          id: "img",
          name: "a.png",
          size: 1,
          content: { data: "aa", mimeType: "image/png", uri: "a.png" },
        },
      ],
      retainedAttachments: false,
      pendingSessionId: null,
      returnTo: null,
      sendInFlight: false,
      updatedAt: 0,
    };
    expect(draftHasContent(draft)).toBe(true);
    expect(sameDraftScope(draft, { projectId: "p1", taskId: null })).toBe(true);
    expect(sameDraftScope(draft, { projectId: "p1", taskId: "t1" })).toBe(
      false,
    );
  });

  it("does not persist image-only drafts across restart", () => {
    const id = useDraftSessionsStore
      .getState()
      .ensureEmptyDraft({ projectId: "p1", taskId: null });
    useDraftSessionsStore.getState().updateContent(id, {
      images: [
        {
          id: "img",
          name: "a.png",
          size: 1,
          content: { data: "aa", mimeType: "image/png", uri: "a.png" },
        },
      ],
    });
    expect(draftHasContent(useDraftSessionsStore.getState().drafts[0]!)).toBe(
      true,
    );
    flushDebouncedPersistStorage();

    const raw = window.localStorage.getItem(SESSION_DRAFTS_STORAGE_KEY);
    const parsed = JSON.parse(raw ?? '{"state":{"drafts":[]}}') as {
      state: { drafts: unknown[] };
    };
    expect(parsed.state.drafts).toEqual([]);
  });

  it("strips pendingSessionId when rehydrating so mid-send binds cannot zombie", async () => {
    useDraftSessionsStore.getState().clear();
    flushDebouncedPersistStorage();
    window.localStorage.setItem(
      SESSION_DRAFTS_STORAGE_KEY,
      JSON.stringify({
        state: {
          drafts: [
            {
              id: "draft-bound",
              projectId: "p1",
              taskId: null,
              text: "in flight",
              images: [],
              retainedAttachments: false,
              pendingSessionId: "s-dead",
              returnTo: null,
              updatedAt: 1,
            },
          ],
        },
        version: 0,
      }),
    );
    await useDraftSessionsStore.persist.rehydrate();
    expect(useDraftSessionsStore.getState().drafts).toEqual([
      expect.objectContaining({
        id: "draft-bound",
        text: "in flight",
        pendingSessionId: null,
      }),
    ]);
  });

  it("persists typed drafts to localStorage without images or empty rows", () => {
    const empty = useDraftSessionsStore
      .getState()
      .ensureEmptyDraft({ projectId: "p1", taskId: null });
    const typed = useDraftSessionsStore
      .getState()
      .ensureEmptyDraft({ projectId: "p1", taskId: null });
    expect(typed).toBe(empty);
    useDraftSessionsStore.getState().updateContent(typed, {
      text: "sidebar title",
      images: [
        {
          id: "img",
          name: "a.png",
          size: 1,
          content: { data: "aa", mimeType: "image/png", uri: "a.png" },
        },
      ],
    });
    const leftoverEmpty = useDraftSessionsStore
      .getState()
      .ensureEmptyDraft({ projectId: "p1", taskId: null });
    expect(leftoverEmpty).not.toBe(typed);
    flushDebouncedPersistStorage();

    const raw = window.localStorage.getItem(SESSION_DRAFTS_STORAGE_KEY);
    expect(raw).not.toBeNull();
    const parsed = JSON.parse(raw!) as {
      state: { drafts: Array<{ id: string; text: string; images: unknown[] }> };
    };
    expect(parsed.state.drafts).toEqual([
      expect.objectContaining({
        id: typed,
        text: "sidebar title",
        images: [],
      }),
    ]);
    expect(
      parsed.state.drafts.some((draft) => draft.id === leftoverEmpty),
    ).toBe(false);
    expect(
      useDraftSessionsStore.getState().drafts.find((d) => d.id === typed)
        ?.images,
    ).toHaveLength(1);
  });

  it("rehydrates typed drafts from localStorage", async () => {
    useDraftSessionsStore.getState().clear();
    flushDebouncedPersistStorage();
    window.localStorage.setItem(
      SESSION_DRAFTS_STORAGE_KEY,
      JSON.stringify({
        state: {
          drafts: [
            {
              id: "draft-1",
              projectId: "p1",
              taskId: "t1",
              text: "come back",
              images: [
                {
                  id: "img",
                  name: "a.png",
                  size: 1,
                  content: { data: "aa", mimeType: "image/png" },
                },
              ],
              pendingSessionId: null,
              updatedAt: 1,
            },
          ],
        },
        version: 0,
      }),
    );
    await useDraftSessionsStore.persist.rehydrate();
    expect(useDraftSessionsStore.getState().drafts).toEqual([
      expect.objectContaining({
        id: "draft-1",
        projectId: "p1",
        taskId: "t1",
        text: "come back",
        images: [],
      }),
    ]);
  });

  it("drops malformed persisted drafts without breaking rehydration", async () => {
    window.localStorage.setItem(
      SESSION_DRAFTS_STORAGE_KEY,
      JSON.stringify({
        state: {
          drafts: [
            { id: "missing-text", projectId: "p1" },
            42,
            {
              id: "valid",
              projectId: "p1",
              taskId: 99,
              text: "recover me",
              returnTo: "corrupt",
              updatedAt: "yesterday",
            },
          ],
        },
        version: 0,
      }),
    );

    await expect(
      useDraftSessionsStore.persist.rehydrate(),
    ).resolves.toBeUndefined();
    expect(useDraftSessionsStore.getState().drafts).toEqual([
      expect.objectContaining({
        id: "valid",
        projectId: "p1",
        taskId: null,
        text: "recover me",
        images: [],
        returnTo: null,
      }),
    ]);
  });

  it("drops a malformed persisted drafts container", async () => {
    window.localStorage.setItem(
      SESSION_DRAFTS_STORAGE_KEY,
      JSON.stringify({ state: { drafts: "corrupt" }, version: 0 }),
    );

    await expect(
      useDraftSessionsStore.persist.rehydrate(),
    ).resolves.toBeUndefined();
    expect(useDraftSessionsStore.getState().drafts).toEqual([]);
  });

  it("clears returnTo entries that point at deleted sessions", () => {
    const id = useDraftSessionsStore
      .getState()
      .ensureEmptyDraft({ projectId: "p1", taskId: null });
    useDraftSessionsStore.getState().updateContent(id, { text: "parked" });
    useDraftSessionsStore.getState().setReturnTo(id, {
      sessionId: "s-gone",
      taskId: "t1",
      projectId: "p1",
    });
    useDraftSessionsStore.getState().clearReturnToForSessions(["s-gone"]);
    expect(
      useDraftSessionsStore.getState().drafts.find((d) => d.id === id)
        ?.returnTo,
    ).toBeNull();
  });

  it("clear() also drops draft composer parks", () => {
    const id = useDraftSessionsStore
      .getState()
      .ensureEmptyDraft({ projectId: "p1", taskId: null });
    useComposerInputStore.getState().setInput(`draft:${id}`, {
      text: "parked",
      images: [],
    });
    useDraftSessionsStore.getState().clear();
    expect(useDraftSessionsStore.getState().drafts).toEqual([]);
    expect(
      useComposerInputStore.getState().byKey[`draft:${id}`],
    ).toBeUndefined();
  });

  it("keeps a draft while sendInFlight so discard and empty reuse cannot race session creation", () => {
    const id = useDraftSessionsStore
      .getState()
      .ensureEmptyDraft({ projectId: "p1", taskId: null });
    useDraftSessionsStore.getState().beginSend(id);
    expect(
      useDraftSessionsStore.getState().drafts.find((d) => d.id === id)
        ?.sendInFlight,
    ).toBe(true);

    useDraftSessionsStore.getState().discardIfEmpty(id);
    expect(useDraftSessionsStore.getState().drafts.map((d) => d.id)).toEqual([
      id,
    ]);

    const other = useDraftSessionsStore
      .getState()
      .ensureEmptyDraft({ projectId: "p1", taskId: null });
    expect(other).not.toBe(id);

    useDraftSessionsStore.getState().endSend(id);
    expect(
      useDraftSessionsStore.getState().drafts.find((d) => d.id === id)
        ?.sendInFlight,
    ).toBe(false);
    useDraftSessionsStore.getState().discardIfEmpty(id);
    expect(useDraftSessionsStore.getState().drafts.map((d) => d.id)).toEqual([
      other,
    ]);
  });
});
