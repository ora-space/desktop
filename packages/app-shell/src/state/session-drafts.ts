import type * as acp from "@agentclientprotocol/sdk";
import type { JSONContent } from "@tiptap/core";
import type { TaskWorkspaceMode } from "@ora/contracts";
import { useDraftSessionsStore } from "./stores/draft-sessions-store";
import type { DraftScope } from "./stores/draft-sessions-store";
import { useComposerInputStore } from "./stores/composer-input-store";
import { useComposerPluginSelectionStore } from "./stores/composer-plugin-selection-store";
import { useUiStore } from "./stores/ui-store";
import { useWorkflowStore } from "./stores/workflow-store";
import { useWorkspaceSelectionStore } from "./stores/workspace-selection-store";
import type { WorkspaceCreateFocus } from "./stores/workspace-selection-store";
import type { WorkspaceSelection } from "./stores/sanitize-workspace-selection";

/**
 * Thrown when a first send is abandoned (Stop / navigated away) so the
 * composer's reject handler can put the message back without treating it as a
 * hard failure.
 */
export class DraftSendAbandonedError extends Error {
  constructor() {
    super("draft send abandoned");
    this.name = "DraftSendAbandonedError";
  }
}

/**
 * Warm session id an in-flight first send adopted, keyed by the conversation
 * key at submit time (`draft:…`, `task:…`, or `__none__`). Composer hard-fail
 * restore uses this so a later navigate to an unrelated chat cannot receive
 * the failed message, while the adopted session (or a recovered draft) still can.
 */
const adoptedSessionBySendKey = new Map<string, string>();

/** Records which warm session a first send selected into. */
export function noteComposerSendAdoptedSession(
  sendKey: string,
  sessionId: string,
): void {
  adoptedSessionBySendKey.set(sendKey, sessionId);
}

/** Drops the adoption mark once the composer catch (or success) has settled. */
export function clearComposerSendAdoption(sendKey: string): void {
  adoptedSessionBySendKey.delete(sendKey);
}

/** Warm id this send's open first send moved onto, if any. */
export function composerSendAdoptedSession(
  sendKey: string,
): string | undefined {
  return adoptedSessionBySendKey.get(sendKey);
}

/** Test helper so adoption marks cannot leak across cases. */
export function resetComposerSendAdoptionsForTests(): void {
  adoptedSessionBySendKey.clear();
}

/** Returns decoded bytes for a base64 image without materializing another byte array. */
function base64ByteLength(data: string): number {
  const compact = data.replace(/\s/gu, "");
  if (compact.length === 0) return 0;
  const padding = compact.endsWith("==") ? 2 : compact.endsWith("=") ? 1 : 0;
  return Math.max(0, Math.floor((compact.length * 3) / 4) - padding);
}

/** Expands the ancestors needed to keep a project-root or worktree draft visible. */
function expandDraftScope(scope: DraftScope): void {
  // A staged session restore owns layout until it commits; expanding here would
  // reopen rows the user had collapsed before quit.
  if (useWorkspaceSelectionStore.getState().pendingRestore !== null) return;
  useUiStore.getState().expandProject(scope.projectId);
  if (scope.taskId !== null) useUiStore.getState().expandTask(scope.taskId);
}

/** Parks send payload onto a draft so Stop/abandon can restore the composer. */
export function reparkDraftComposerContent(args: {
  draftId: string;
  text: string;
  images?: acp.ImageContent[];
  /** TipTap JSON so chips survive abandon/fail when the process is still alive. */
  doc?: JSONContent;
}): void {
  const { draftId, text, images = [], doc } = args;
  const parkedImages = images.map((content, index) => ({
    id: `recovered-${index}`,
    name: content.uri ?? `image-${index + 1}`,
    size: base64ByteLength(content.data),
    content,
  }));
  useDraftSessionsStore.getState().updateContent(draftId, {
    text,
    images: parkedImages,
  });
  useComposerInputStore.getState().setInput(`draft:${draftId}`, {
    text,
    images: parkedImages,
    ...(doc !== undefined ? { doc } : {}),
  });
}

/** Loaded tree shape `resolveNewChatScope` validates New-chat targets against. */
type NewChatTree = {
  projects: readonly { id: string }[];
  tasks: readonly {
    id: string;
    projectId: string;
    workspaceMode: TaskWorkspaceMode;
  }[];
};

/**
 * Picks where global New chat / Ctrl+N should create a draft.
 *
 * Prefers the last tree create-focus (project/worktree row click), then the
 * live selection's project/task, then the first project. Returns null only
 * when the workspace has no projects at all.
 *
 * When `tree` is provided, a create-focus or selection whose project vanished
 * is ignored, a missing worktree is demoted to project-root, and a project-root
 * task is demoted to a direct (project-level) draft. The sidebar only renders a
 * task-scoped draft under a worktree branch, so New chat must never keep a
 * project-root task id — that would create a draft no tree row can show.
 */
export function resolveNewChatScope(
  createFocus: WorkspaceCreateFocus | null,
  selection: WorkspaceSelection,
  firstProjectId: string | null,
  tree?: NewChatTree,
): DraftScope | null {
  const focus = clampCreateFocus(createFocus, tree);
  if (focus !== null) {
    return {
      projectId: focus.projectId,
      taskId: focus.taskId,
    };
  }
  if (selection.projectId !== null) {
    const scope = scopeDraftTarget(
      { projectId: selection.projectId, taskId: selection.taskId },
      tree,
    );
    if (scope !== null) return scope;
  }
  if (firstProjectId !== null) {
    return { projectId: firstProjectId, taskId: null };
  }
  return null;
}

/**
 * Validates a {project, task} target against the loaded tree and returns the
 * draft scope New chat should create under. A missing/mismatched/project-root
 * task demotes to a project-root (direct) draft — worktree drafts render under
 * the task branch, so a project-root task id would orphan the row. Returns null
 * only when the project itself is gone, so the caller falls through to the next
 * preference instead of creating a draft under a deleted project.
 */
function scopeDraftTarget(
  scope: { projectId: string; taskId: string | null },
  tree: NewChatTree | undefined,
): { projectId: string; taskId: string | null } | null {
  if (tree === undefined) return scope;
  if (!tree.projects.some((project) => project.id === scope.projectId)) {
    return null;
  }
  if (scope.taskId === null) return scope;
  const task = tree.tasks.find((item) => item.id === scope.taskId);
  if (
    task === undefined ||
    task.projectId !== scope.projectId ||
    task.workspaceMode === "project_root"
  ) {
    return { projectId: scope.projectId, taskId: null };
  }
  return scope;
}

/** Drops or demotes create-focus that no longer matches the loaded tree. */
function clampCreateFocus(
  createFocus: WorkspaceCreateFocus | null,
  tree: NewChatTree | undefined,
): WorkspaceCreateFocus | null {
  if (createFocus === null) return null;
  if (tree === undefined) return createFocus;
  const scoped = scopeDraftTarget(createFocus, tree);
  return scoped === null ? null : scoped;
}

/**
 * Opens a new-chat surface: reuse the unused empty draft for this scope, or
 * mint one, then select it and expand its ancestors so the muted leaf is
 * visible immediately.
 */
export function startSessionDraft(scope: DraftScope): string {
  const previous = useWorkspaceSelectionStore.getState().selection;
  const id = useDraftSessionsStore.getState().ensureEmptyDraft(scope);
  // Re-clicking New on the same empty draft must keep the original returnTo;
  // only record a destination when actually leaving another surface.
  if (previous.draftId !== id) {
    useDraftSessionsStore
      .getState()
      .setReturnTo(id, resolveDraftReturnTo(previous));
  }
  useWorkspaceSelectionStore
    .getState()
    .selectDraft(id, scope.taskId, scope.projectId);
  expandDraftScope(scope);
  return id;
}

/**
 * Session to restore when × dismisses an unused draft. Leaving a live session
 * records that session; leaving another draft inherits its origin so a chain of
 * New clicks still returns to the chat the user started from.
 */
function resolveDraftReturnTo(previous: {
  sessionId: string | null;
  taskId: string | null;
  projectId: string | null;
  draftId: string | null;
}): { sessionId: string; taskId: string | null; projectId: string } | null {
  if (previous.sessionId !== null && previous.projectId !== null) {
    return {
      sessionId: previous.sessionId,
      taskId: previous.taskId,
      projectId: previous.projectId,
    };
  }
  if (previous.draftId === null) return null;
  return (
    useDraftSessionsStore
      .getState()
      .drafts.find((candidate) => candidate.id === previous.draftId)
      ?.returnTo ?? null
  );
}

/**
 * Opens the live session a draft is committing into, once bind has pointed it
 * at a warm id. Used when the muted row is clicked during attach.
 */
export function selectBoundDraftSession(draft: {
  projectId: string;
  taskId: string | null;
  pendingSessionId: string;
}): void {
  if (draft.taskId !== null) {
    useWorkspaceSelectionStore
      .getState()
      .selectSession(draft.pendingSessionId, draft.taskId, draft.projectId);
  } else {
    useWorkspaceSelectionStore
      .getState()
      .selectSessionBeforeTask(draft.pendingSessionId, draft.projectId);
  }
  expandDraftScope(draft);
}

/**
 * Rolls a failed first-send back onto its draft: clear the dead warm bind,
 * re-park the message for the composer, and select the muted row again so ×
 * works and a retry can warm a fresh session.
 *
 * `boundSessionId` must be the warm id this send bound to — not whatever the
 * selection happens to point at after the user navigated away mid-attach.
 */
export function recoverFailedDraftSend(args: {
  draftId: string;
  projectId: string;
  taskId: string | null;
  text: string;
  images?: acp.ImageContent[];
  doc?: JSONContent;
  boundSessionId: string;
}): void {
  const {
    draftId,
    projectId,
    taskId,
    text,
    images = [],
    doc,
    boundSessionId,
  } = args;
  useDraftSessionsStore.getState().restoreForRetry(draftId, {
    projectId,
    taskId,
  });
  reparkDraftComposerContent({ draftId, text, images, doc });
  // Plugin picks and workflow state were rekeyed onto the warm id; move them
  // back so a retry on the draft surface keeps the same constellation.
  const draftKey = `draft:${draftId}`;
  useComposerPluginSelectionStore.getState().rekey(boundSessionId, draftKey);
  useWorkflowStore.getState().rekey(boundSessionId, draftKey);
  useComposerInputStore.getState().clear(boundSessionId);
  useWorkspaceSelectionStore.getState().selectDraft(draftId, taskId, projectId);
  expandDraftScope({ projectId, taskId });
}

/**
 * Dismisses a draft from the tree.
 *
 * A draft that is only still visible because it is binding onto the selected
 * session is removed without touching selection — the live chat stays put.
 * An ordinary selected draft falls back to a sibling, else the session the
 * user left when opening it, else the parent project or worktree.
 */
export function dismissSessionDraft(id: string): void {
  const draftStore = useDraftSessionsStore.getState();
  const draft = draftStore.drafts.find((candidate) => candidate.id === id);
  // In-flight first send still needs this row for repark; × is hidden for the
  // same reason, but refuse here so callers cannot race the warm handshake.
  if (draft === undefined || draft.sendInFlight) return;
  const selection = useWorkspaceSelectionStore.getState().selection;
  const boundToCurrentSession =
    draft.pendingSessionId !== null &&
    selection.sessionId === draft.pendingSessionId;
  const wasSelectedDraft = selection.draftId === id;
  const returnTo = draft.returnTo;
  draftStore.remove(id);
  // Bound rows shadow the live session; dropping them must not navigate away.
  if (boundToCurrentSession || !wasSelectedDraft) return;

  const sibling = useDraftSessionsStore
    .getState()
    .drafts.filter(
      (candidate) =>
        candidate.projectId === draft.projectId &&
        candidate.taskId === draft.taskId,
    )
    .sort(
      (left, right) =>
        right.updatedAt - left.updatedAt || left.id.localeCompare(right.id),
    )[0];
  if (sibling !== undefined) {
    useWorkspaceSelectionStore
      .getState()
      .selectDraft(sibling.id, sibling.taskId, sibling.projectId);
    return;
  }
  if (returnTo !== null) {
    if (returnTo.taskId !== null) {
      useWorkspaceSelectionStore
        .getState()
        .selectSession(returnTo.sessionId, returnTo.taskId, returnTo.projectId);
    } else {
      useWorkspaceSelectionStore
        .getState()
        .selectSessionBeforeTask(returnTo.sessionId, returnTo.projectId);
    }
    expandDraftScope(returnTo);
    return;
  }
  if (draft.taskId !== null) {
    useWorkspaceSelectionStore
      .getState()
      .selectTask(draft.taskId, draft.projectId);
    return;
  }
  useWorkspaceSelectionStore.getState().selectProject(draft.projectId);
}
