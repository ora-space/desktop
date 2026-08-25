import { create } from "zustand";
import { persist } from "zustand/middleware";
import type * as acp from "@agentclientprotocol/sdk";
import { useComposerInputStore } from "./composer-input-store";
import { createDebouncedJSONStorage } from "./debounced-json-storage";

/** One image parked on a client-only session draft (memory only; not written to disk). */
export interface DraftImage {
  id: string;
  name: string;
  size: number;
  content: acp.ImageContent;
}

export interface SessionDraft {
  id: string;
  projectId: string;
  taskId: string | null;
  text: string;
  images: DraftImage[];
  /**
   * True after the user attached images in this process. Image bytes are never
   * written to disk; restart drops attachment-only drafts and restores typed
   * drafts without their images.
   */
  retainedAttachments: boolean;
  /**
   * Warm session this draft is committing into. Kept until that id appears in
   * the persisted session list so the muted row stays selected during attach.
   */
  pendingSessionId: string | null;
  /**
   * Session the user left when opening this draft. Dismissing with × restores
   * it so an unused new-chat does not dump them on the project landing.
   */
  returnTo: DraftReturnTo | null;
  /**
   * True from first-send start until the send settles. Hides × during the warm
   * handshake so dismissing cannot delete the draft out from under repark.
   */
  sendInFlight: boolean;
  updatedAt: number;
}

/** Where × should navigate after dismissing an unused draft. */
export interface DraftReturnTo {
  sessionId: string;
  taskId: string | null;
  projectId: string;
}

export interface DraftScope {
  projectId: string;
  taskId: string | null;
}

/**
 * Tree placement for a draft, ignoring composer text/images so sidebar parents
 * can skip re-rendering on every keystroke.
 */
export interface DraftPlacement {
  id: string;
  projectId: string;
  taskId: string | null;
  pendingSessionId: string | null;
}

interface DraftSessionsState {
  drafts: SessionDraft[];
  /**
   * Returns the empty draft for this scope, creating one when none is sitting
   * unused. Contentful drafts are left alone so a typed-and-left row can sit
   * beside a new empty composer.
   */
  ensureEmptyDraft: (scope: DraftScope) => string;
  /** Replaces composer text and/or images on one draft. */
  updateContent: (
    id: string,
    patch: { text?: string; images?: DraftImage[] },
  ) => void;
  /** Records the live session × should restore when this draft is dismissed. */
  setReturnTo: (id: string, returnTo: DraftReturnTo | null) => void;
  /** Points a draft at the warm session id its first send is attaching. */
  bindToSession: (id: string, sessionId: string) => void;
  /**
   * Marks a draft's first send as in flight so × cannot delete it during the
   * warm handshake (before bind).
   */
  beginSend: (id: string) => void;
  /** Clears the in-flight mark when the first send settles or is abandoned. */
  endSend: (id: string) => void;
  /**
   * Clears a failed bind so the muted row is dismissible again and a retry can
   * warm a fresh session id.
   */
  unbindFromSession: (id: string) => void;
  /**
   * After a failed first send, clears the dead bind and reseats the draft under
   * the task that prepare may already have created.
   */
  restoreForRetry: (id: string, scope: DraftScope) => void;
  /** Removes drafts whose pending session has now been persisted. */
  removeCommitted: (sessionIds: Iterable<string>) => void;
  /** Drops a draft regardless of content (explicit × or a finished commit). */
  remove: (id: string) => void;
  /** Drops every draft under a project (used when that project is deleted). */
  removeForProject: (projectId: string) => void;
  /** Drops every draft under a worktree task (used when that task is deleted). */
  removeForTask: (taskId: string) => void;
  /** Drops drafts bound to any of the given warm/persisted session ids. */
  removeForSessions: (sessionIds: Iterable<string>) => void;
  /**
   * Clears returnTo entries that point at deleted sessions so × cannot select
   * a ghost chat after the destination was removed.
   */
  clearReturnToForSessions: (sessionIds: Iterable<string>) => void;
  /**
   * Drops a draft only when it still has no composer content and is not in
   * the middle of a send. Leaving an empty new-chat surface uses this.
   */
  discardIfEmpty: (id: string) => void;
  /** Test and teardown helper so drafts cannot leak across cases. */
  clear: () => void;
}

export const SESSION_DRAFTS_STORAGE_KEY = "ora.session-drafts.v1";

/** True when leaving this draft should keep it in the tree. */
export function draftHasContent(draft: SessionDraft): boolean {
  return (
    draft.text.trim().length > 0 ||
    draft.images.length > 0 ||
    draft.retainedAttachments
  );
}

/** True when two drafts belong to the same main or isolated Workspace surface. */
export function sameDraftScope(
  draft: SessionDraft,
  scope: DraftScope,
): boolean {
  return draft.projectId === scope.projectId && draft.taskId === scope.taskId;
}

/**
 * Sidebar label for a draft: the first line of typed text, otherwise the
 * empty-session fallback. CSS truncation handles length; this only picks the
 * line so a parked row still reads as the message the user started.
 */
export function draftSidebarTitle(text: string, fallback: string): string {
  const line = text.trim().split("\n", 1)[0] ?? "";
  const normalized = line.replace(/\s+/gu, " ").trim();
  return normalized.length > 0 ? normalized : fallback;
}

/** Placement list used by the sidebar tree (identity-stable across typing). */
export function draftPlacements(drafts: SessionDraft[]): DraftPlacement[] {
  return drafts.map((draft) => ({
    id: draft.id,
    projectId: draft.projectId,
    taskId: draft.taskId,
    pendingSessionId: draft.pendingSessionId,
  }));
}

/** True when two placement lists describe the same tree structure. */
export function draftPlacementsEqual(
  left: DraftPlacement[],
  right: DraftPlacement[],
): boolean {
  if (left.length !== right.length) return false;
  return left.every((placement, index) => {
    const candidate = right[index]!;
    return (
      placement.id === candidate.id &&
      placement.projectId === candidate.projectId &&
      placement.taskId === candidate.taskId &&
      placement.pendingSessionId === candidate.pendingSessionId
    );
  });
}

/** Clears composer parks for the given draft ids. */
function clearDraftComposerKeys(draftIds: Iterable<string>): void {
  useComposerInputStore
    .getState()
    .clearKeys([...draftIds].map((id) => `draft:${id}`));
}

/**
 * Disk shape for drafts: keep typed rows only. In-flight binds and image bytes
 * are process-local — restoring a pendingSessionId across restart left
 * undismissable zombies pointing at dead warm ids, and attachment-only rows
 * came back as empty "New session" shells.
 */
function sanitizeDraftsForDisk(drafts: unknown): SessionDraft[] {
  if (!Array.isArray(drafts)) return [];
  const sanitized: SessionDraft[] = [];
  for (const draft of drafts) {
    if (typeof draft !== "object" || draft === null || Array.isArray(draft)) {
      continue;
    }
    const candidate = draft as Record<string, unknown>;
    if (
      typeof candidate.id !== "string" ||
      candidate.id.length === 0 ||
      typeof candidate.projectId !== "string" ||
      candidate.projectId.length === 0 ||
      typeof candidate.text !== "string" ||
      candidate.text.trim().length === 0
    ) {
      continue;
    }
    sanitized.push({
      id: candidate.id,
      projectId: candidate.projectId,
      taskId: typeof candidate.taskId === "string" ? candidate.taskId : null,
      text: candidate.text,
      images: [],
      retainedAttachments: false,
      pendingSessionId: null,
      returnTo: sanitizeReturnTo(candidate.returnTo),
      sendInFlight: false,
      updatedAt:
        typeof candidate.updatedAt === "number" &&
        Number.isFinite(candidate.updatedAt)
          ? candidate.updatedAt
          : Date.now(),
    });
  }
  return sanitized;
}

/** Drops corrupt returnTo payloads instead of resurrecting a broken selection. */
function sanitizeReturnTo(returnTo: unknown): DraftReturnTo | null {
  if (
    typeof returnTo !== "object" ||
    returnTo === null ||
    Array.isArray(returnTo) ||
    !("sessionId" in returnTo) ||
    typeof returnTo.sessionId !== "string" ||
    returnTo.sessionId.length === 0 ||
    !("projectId" in returnTo) ||
    typeof returnTo.projectId !== "string" ||
    returnTo.projectId.length === 0
  ) {
    return null;
  }
  return {
    sessionId: returnTo.sessionId,
    projectId: returnTo.projectId,
    taskId:
      "taskId" in returnTo &&
      (returnTo.taskId === null || typeof returnTo.taskId === "string")
        ? returnTo.taskId
        : null,
  };
}

/** Partitions drafts whose pending session has entered the supplied session set. */
function removeBoundDrafts(
  drafts: SessionDraft[],
  sessionIds: Iterable<string>,
): { remaining: SessionDraft[]; removed: SessionDraft[] } {
  const committed = new Set(sessionIds);
  if (committed.size === 0) return { remaining: drafts, removed: [] };
  const remaining: SessionDraft[] = [];
  const removed: SessionDraft[] = [];
  for (const draft of drafts) {
    if (
      draft.pendingSessionId !== null &&
      committed.has(draft.pendingSessionId)
    ) {
      removed.push(draft);
    } else {
      remaining.push(draft);
    }
  }
  return { remaining, removed };
}

/** Client-only drafts for chats that have not been attached to a Task yet. */
export const useDraftSessionsStore = create<DraftSessionsState>()(
  persist(
    (set, get) => ({
      drafts: [],
      ensureEmptyDraft: (scope) => {
        const existing = get().drafts.find(
          (draft) =>
            sameDraftScope(draft, scope) &&
            !draftHasContent(draft) &&
            draft.pendingSessionId === null &&
            !draft.sendInFlight,
        );
        if (existing !== undefined) return existing.id;
        const id = crypto.randomUUID();
        set((state) => ({
          drafts: [
            ...state.drafts,
            {
              id,
              projectId: scope.projectId,
              taskId: scope.taskId,
              text: "",
              images: [],
              retainedAttachments: false,
              pendingSessionId: null,
              returnTo: null,
              sendInFlight: false,
              updatedAt: Date.now(),
            },
          ],
        }));
        return id;
      },
      updateContent: (id, patch) =>
        set((state) => {
          const draft = state.drafts.find((candidate) => candidate.id === id);
          if (draft === undefined) return state;
          const nextText = patch.text ?? draft.text;
          const nextImages = patch.images ?? draft.images;
          const nextRetained =
            patch.images !== undefined
              ? patch.images.length > 0
              : draft.retainedAttachments || draft.images.length > 0;
          if (
            nextText === draft.text &&
            nextImages === draft.images &&
            nextRetained === draft.retainedAttachments
          ) {
            return state;
          }
          return {
            drafts: state.drafts.map((candidate) =>
              candidate.id === id
                ? {
                    ...candidate,
                    text: nextText,
                    images: nextImages,
                    retainedAttachments: nextRetained,
                    updatedAt: Date.now(),
                  }
                : candidate,
            ),
          };
        }),
      setReturnTo: (id, returnTo) =>
        set((state) => {
          const draft = state.drafts.find((candidate) => candidate.id === id);
          if (draft === undefined) return state;
          const next = sanitizeReturnTo(returnTo);
          if (
            draft.returnTo?.sessionId === next?.sessionId &&
            draft.returnTo?.taskId === next?.taskId &&
            draft.returnTo?.projectId === next?.projectId
          ) {
            return state;
          }
          return {
            drafts: state.drafts.map((candidate) =>
              candidate.id === id
                ? { ...candidate, returnTo: next, updatedAt: Date.now() }
                : candidate,
            ),
          };
        }),
      bindToSession: (id, sessionId) =>
        set((state) => {
          const draft = state.drafts.find((candidate) => candidate.id === id);
          if (draft === undefined || draft.pendingSessionId === sessionId) {
            return state;
          }
          return {
            drafts: state.drafts.map((candidate) =>
              candidate.id === id
                ? { ...candidate, pendingSessionId: sessionId }
                : candidate,
            ),
          };
        }),
      beginSend: (id) =>
        set((state) => {
          const draft = state.drafts.find((candidate) => candidate.id === id);
          if (draft === undefined || draft.sendInFlight) return state;
          return {
            drafts: state.drafts.map((candidate) =>
              candidate.id === id
                ? { ...candidate, sendInFlight: true }
                : candidate,
            ),
          };
        }),
      endSend: (id) =>
        set((state) => {
          const draft = state.drafts.find((candidate) => candidate.id === id);
          if (draft === undefined || !draft.sendInFlight) return state;
          return {
            drafts: state.drafts.map((candidate) =>
              candidate.id === id
                ? { ...candidate, sendInFlight: false }
                : candidate,
            ),
          };
        }),
      unbindFromSession: (id) =>
        set((state) => {
          const draft = state.drafts.find((candidate) => candidate.id === id);
          if (draft === undefined || draft.pendingSessionId === null) {
            return state;
          }
          return {
            drafts: state.drafts.map((candidate) =>
              candidate.id === id
                ? { ...candidate, pendingSessionId: null }
                : candidate,
            ),
          };
        }),
      restoreForRetry: (id, scope) =>
        set((state) => {
          const draft = state.drafts.find((candidate) => candidate.id === id);
          if (draft === undefined) return state;
          if (
            draft.pendingSessionId === null &&
            draft.projectId === scope.projectId &&
            draft.taskId === scope.taskId
          ) {
            return state;
          }
          return {
            drafts: state.drafts.map((candidate) =>
              candidate.id === id
                ? {
                    ...candidate,
                    projectId: scope.projectId,
                    taskId: scope.taskId,
                    pendingSessionId: null,
                    sendInFlight: false,
                    updatedAt: Date.now(),
                  }
                : candidate,
            ),
          };
        }),
      removeCommitted: (sessionIds) => {
        const { remaining, removed } = removeBoundDrafts(
          get().drafts,
          sessionIds,
        );
        if (removed.length === 0) return;
        clearDraftComposerKeys(removed.map((draft) => draft.id));
        set({ drafts: remaining });
      },
      remove: (id) => {
        useComposerInputStore.getState().clear(`draft:${id}`);
        set((state) => {
          if (!state.drafts.some((draft) => draft.id === id)) return state;
          return {
            drafts: state.drafts.filter((draft) => draft.id !== id),
          };
        });
      },
      removeForProject: (projectId) => {
        const removing = get().drafts.filter(
          (draft) => draft.projectId === projectId,
        );
        if (removing.length === 0) return;
        clearDraftComposerKeys(removing.map((draft) => draft.id));
        set((state) => ({
          drafts: state.drafts.filter((draft) => draft.projectId !== projectId),
        }));
      },
      removeForTask: (taskId) => {
        const removing = get().drafts.filter(
          (draft) => draft.taskId === taskId,
        );
        if (removing.length === 0) return;
        clearDraftComposerKeys(removing.map((draft) => draft.id));
        set((state) => ({
          drafts: state.drafts.filter((draft) => draft.taskId !== taskId),
        }));
      },
      removeForSessions: (sessionIds) => {
        const { remaining, removed } = removeBoundDrafts(
          get().drafts,
          sessionIds,
        );
        if (removed.length === 0) return;
        clearDraftComposerKeys(removed.map((draft) => draft.id));
        set({ drafts: remaining });
      },
      clearReturnToForSessions: (sessionIds) => {
        const deleted = new Set(sessionIds);
        if (deleted.size === 0) return;
        set((state) => {
          let changed = false;
          const drafts = state.drafts.map((draft) => {
            if (
              draft.returnTo === null ||
              !deleted.has(draft.returnTo.sessionId)
            ) {
              return draft;
            }
            changed = true;
            return { ...draft, returnTo: null, updatedAt: Date.now() };
          });
          return changed ? { drafts } : state;
        });
      },
      discardIfEmpty: (id) => {
        const draft = get().drafts.find((candidate) => candidate.id === id);
        if (
          draft === undefined ||
          draftHasContent(draft) ||
          draft.pendingSessionId !== null ||
          draft.sendInFlight
        ) {
          return;
        }
        get().remove(id);
      },
      clear: () => {
        clearDraftComposerKeys(get().drafts.map((draft) => draft.id));
        set({ drafts: [] });
      },
    }),
    {
      name: SESSION_DRAFTS_STORAGE_KEY,
      // Keystroke parks coalesce; pagehide / visibility flush for durability.
      storage: createDebouncedJSONStorage(),
      // Empty composers are recreate-on-demand; only typed / mid-send rows survive.
      partialize: (state) => ({
        drafts: sanitizeDraftsForDisk(state.drafts),
      }),
      merge: (persisted, current) => {
        const slice =
          typeof persisted === "object" && persisted !== null
            ? (persisted as Record<string, unknown>)
            : undefined;
        return {
          ...current,
          drafts: sanitizeDraftsForDisk(slice?.drafts),
        };
      },
    },
  ),
);
