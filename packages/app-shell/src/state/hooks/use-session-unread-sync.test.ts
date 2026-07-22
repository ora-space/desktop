import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { createChatStore, type ChatStore, type SessionConversation } from "@ora/chat";
import { createMockClient, createMockClientState } from "../../test/mock-client";
import { useSessionUnreadSync } from "./use-session-unread-sync";
import { useUnreadSessionsStore } from "../stores/unread-sessions-store";
import { useWorkspaceSelectionStore } from "../stores/workspace-selection-store";

/** Builds an idle conversation, overriding only the fields a test drives. */
function conversation(overrides: Partial<SessionConversation> = {}): SessionConversation {
  return {
    turns: [],
    isLoaded: false,
    isLoading: false,
    isResponding: false,
    pendingPermissions: [],
    error: null,
    ...overrides,
  };
}

/** A chat store we never prompt - tests write conversation state directly. */
function makeChatStore(): ChatStore {
  return createChatStore(createMockClient(createMockClientState()).session);
}

/** Sets one session's live responding flag, leaving the others untouched. */
function setResponding(store: ChatStore, id: string, isResponding: boolean): void {
  act(() =>
    store.setState((state) => ({
      conversations: { ...state.conversations, [id]: conversation({ isResponding }) },
    })),
  );
}

const isUnread = (id: string) => useUnreadSessionsStore.getState().unread.has(id);
const select = (sessionId: string) =>
  act(() => useWorkspaceSelectionStore.getState().selectSession(sessionId, "t1", "p1"));

beforeEach(() => {
  useUnreadSessionsStore.setState({ unread: new Set() });
  useWorkspaceSelectionStore.getState().clearSelection();
});

describe("useSessionUnreadSync", () => {
  it("marks a session unread when its turn ends while another session is open", () => {
    select("A");
    const store = makeChatStore();
    renderHook(() => useSessionUnreadSync(store));

    setResponding(store, "B", true);
    setResponding(store, "B", false);

    expect(isUnread("B")).toBe(true);
  });

  it("does not mark the open session unread when its own turn ends", () => {
    select("B");
    const store = makeChatStore();
    renderHook(() => useSessionUnreadSync(store));

    setResponding(store, "B", true);
    setResponding(store, "B", false);

    expect(isUnread("B")).toBe(false);
  });

  it("clears the unread mark once the session is opened", () => {
    select("A");
    const store = makeChatStore();
    renderHook(() => useSessionUnreadSync(store));
    setResponding(store, "B", true);
    setResponding(store, "B", false);
    expect(isUnread("B")).toBe(true);

    select("B");

    expect(isUnread("B")).toBe(false);
  });

  it("still flags a turn that was already in flight when the hook mounted", () => {
    select("A");
    const store = makeChatStore();
    // B is mid-turn before the sync starts; finishing it later must still count.
    setResponding(store, "B", true);
    renderHook(() => useSessionUnreadSync(store));

    setResponding(store, "B", false);

    expect(isUnread("B")).toBe(true);
  });

  it("does not flag a session that merely starts responding", () => {
    select("A");
    const store = makeChatStore();
    renderHook(() => useSessionUnreadSync(store));

    setResponding(store, "B", true);

    expect(isUnread("B")).toBe(false);
  });
});
