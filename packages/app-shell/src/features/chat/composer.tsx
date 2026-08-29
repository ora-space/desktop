import type * as acp from "@agentclientprotocol/sdk";
import {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  IconArrowUp,
  IconLoader2,
  IconPhoto,
  IconPlayerStop,
  IconPlus,
  IconX,
} from "@tabler/icons-react";
import { Button } from "@ora/ui";
import {
  ComposerEditor,
  type ComposerEditorHandle,
} from "../editor/composer-editor";
import type { PromptTokenKind } from "@ora/editor/composer";
import {
  EMPTY_COMPOSER_QUERY,
  queryStateFromText,
  queryStatesEqual,
  type ComposerQueryState,
} from "../editor/composer-query";
import type { JSONContent } from "@tiptap/core";
import type { Agent, Skill } from "@ora/contracts";
import { useTranslation } from "react-i18next";
import { ModelSelector } from "./model-selector";
import { PermissionSelector } from "./permission-selector";
import { ComposerActionMenu } from "./composer-action-menu";
import { ImagePreviewDialog } from "./image-preview-dialog";
import {
  buildComposerActions,
  buildComposerFileActions,
  filterComposerActions,
  visibleComposerActions,
  type ComposerAction,
  type ComposerActionGroup,
} from "./composer-actions";
import { SelectedPluginsButton } from "./selected-plugins-button";
import {
  fileMentionMenuStatus,
  fileMentionStatusMessageKey,
  useComposerFileMentions,
} from "./use-composer-file-mentions";
import { useComposerFileContextStore } from "../../state/stores/composer-file-context-store";
import { usePluginInstallStore } from "../../state/stores/plugin-install-store";
import { useComposerPluginSelectionStore } from "../../state/stores/composer-plugin-selection-store";
import { useComposerInputStore } from "../../state/stores/composer-input-store";
import { conversationKeyFor } from "../../state/stores/conversation-key";
import { useDraftSessionsStore } from "../../state/stores/draft-sessions-store";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import {
  clearComposerSendAdoption,
  DraftSendAbandonedError,
  composerSendAdoptedSession,
} from "../../state/session-drafts";
import {
  PLUGIN_CATALOG,
  findPlugin,
  type PluginEntry,
} from "../settings/plugin-catalog";
/** Stable empty array so the store selector below doesn't return a fresh reference every render. */
const EMPTY_PLUGIN_IDS: string[] = [];

interface ComposerProps {
  taskId?: string;
  /** Project checkout used for @ mentions when no task is selected yet. */
  projectId?: string;
  onSend: (text: string, images?: acp.ImageContent[]) => void | Promise<void>;
  onStop?: () => void;
  isResponding: boolean;
  /**
   * True once the agent has produced visible output for the live turn. While the
   * turn is still spinning up (session starting or awaiting the first token) this
   * stays false, which is what splits the send button's stop affordance into a
   * loading spinner and the actual stop icon. The click action is the same in
   * both — only the glyph changes.
   */
  isStreaming?: boolean;
  disabled?: boolean;
  /** Allows the agent/model picker to remain actionable while message composition is blocked. */
  modelSelectorDisabled?: boolean;
  /** Session whose model configuration the selector should display. */
  modelSelectorSessionId?: string;
  placeholder?: string;
  autoFocus?: boolean;
  skills?: Skill[];
  roles?: Agent[];
  availableCommands?: acp.AvailableCommand[];
}

interface ImageAttachment {
  id: string;
  name: string;
  size: number;
  content: acp.ImageContent;
}

const ACCEPTED_IMAGE_TYPES = new Set([
  "image/avif",
  "image/bmp",
  "image/gif",
  "image/jpeg",
  "image/png",
  "image/webp",
]);
const MAX_IMAGE_BYTES = 5 * 1024 * 1024;
const MAX_TOTAL_IMAGE_BYTES = 10 * 1024 * 1024;

/**
 * The chat composer: a rounded input shell wrapping ComposerEditor with an
 * inline send button. Enter sends, Shift+Enter inserts a newline, and the
 * editor auto-grows up to a max height.
 */
export function Composer({
  taskId,
  projectId,
  onSend,
  onStop,
  isResponding,
  isStreaming = false,
  disabled = false,
  modelSelectorDisabled = disabled,
  modelSelectorSessionId,
  placeholder,
  autoFocus = false,
  skills = [],
  roles = [],
  availableCommands = [],
}: ComposerProps) {
  const { t } = useTranslation();
  const [query, setQuery] = useState<ComposerQueryState>(EMPTY_COMPOSER_QUERY);
  const [selectedActionIndex, setSelectedActionIndex] = useState(0);
  const actionHighlightSourceRef = useRef<"keyboard" | "pointer">("keyboard");
  const [menuDismissed, setMenuDismissed] = useState(false);
  const [plusMenuOpen, setPlusMenuOpen] = useState(false);
  const [expandedGroups, setExpandedGroups] = useState<
    ReadonlySet<ComposerActionGroup>
  >(new Set());
  const [attachments, setAttachmentsState] = useState<ImageAttachment[]>([]);
  const attachmentsRef = useRef<ImageAttachment[]>([]);
  const [attachmentError, setAttachmentError] = useState<string | null>(null);
  const installedPluginIds = usePluginInstallStore(
    (state) => state.installedIds,
  );
  // Keyed by conversation, not by task: sibling sessions under one task each keep their own
  // applied plugins, and the composer instance is reused across session switches rather than
  // remounted, so this cannot live in component state. `dispatchSend` rekeys a pre-session
  // conversation onto its real session id, which is what carries the picks across a first send.
  const conversationKey = useWorkspaceSelectionStore((state) =>
    conversationKeyFor(state.selection),
  );
  const draftId = useWorkspaceSelectionStore(
    (state) => state.selection.draftId,
  );
  const selectedSessionId = useWorkspaceSelectionStore(
    (state) => state.selection.sessionId,
  );
  const selectedPluginIds = useComposerPluginSelectionStore(
    (state) =>
      state.selectedIdsByConversation[conversationKey] ?? EMPTY_PLUGIN_IDS,
  );
  const addSelectedPlugin = useComposerPluginSelectionStore(
    (state) => state.addPlugin,
  );
  const removeSelectedPlugin = useComposerPluginSelectionStore(
    (state) => state.removePlugin,
  );
  const selectedPlugins = useMemo(
    () =>
      selectedPluginIds
        .map(findPlugin)
        .filter((plugin): plugin is PluginEntry => plugin !== undefined),
    [selectedPluginIds],
  );
  // Only plugins the user actually installed and hasn't already applied
  // show up in "+" — picking one removes it from the menu until it is removed below.
  const composerPlugins = useMemo(
    () =>
      PLUGIN_CATALOG.filter(
        (plugin) =>
          installedPluginIds.includes(plugin.id) &&
          !selectedPluginIds.includes(plugin.id),
      ),
    [installedPluginIds, selectedPluginIds],
  );
  const [previewedAttachment, setPreviewedAttachment] =
    useState<ImageAttachment | null>(null);
  const composerRef = useRef<HTMLDivElement>(null);
  const editorRef = useRef<ComposerEditorHandle>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const footerRef = useRef<HTMLDivElement>(null);
  const leftControlsRef = useRef<HTMLDivElement>(null);
  const rightControlsRef = useRef<HTMLDivElement>(null);
  const fullRightControlsWidthRef = useRef<number | null>(null);
  const [showModelSelector, setShowModelSelector] = useState(true);
  const actionOptionRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const actionMenuId = useId();
  // Bumped on every submit so an older send's reject cannot restore text over a
  // newer attempt (Stop during handshake, then type and send again).
  const submitGenerationRef = useRef(0);
  const bindFileContextDelivery = useComposerFileContextStore(
    (state) => state.bindDelivery,
  );

  /** Keeps async attachment work and React state on the same latest array. */
  const replaceAttachments = useCallback((next: ImageAttachment[]) => {
    const current = attachmentsRef.current;
    if (
      current.length === next.length &&
      current.every((item, index) => item.id === next[index]?.id)
    ) {
      attachmentsRef.current = next;
      return;
    }
    attachmentsRef.current = next;
    setAttachmentsState(next);
  }, []);

  /**
   * Parks unsent text/images/doc on the current conversation key so switching
   * sessions (or drafts) can restore them. TipTap `doc` stays in memory so
   * chips round-trip with their kind. Also mirrors typed text onto the draft
   * store while the surface is still a client-only draft (sidebar title).
   */
  const persistComposerInput = useCallback(
    (text: string, nextImages?: ImageAttachment[], doc?: JSONContent) => {
      const images = nextImages ?? attachmentsRef.current;
      const parkedDoc = doc ?? editorRef.current?.getJSON();
      useComposerInputStore.getState().setInput(conversationKey, {
        text,
        images,
        ...(parkedDoc !== undefined ? { doc: parkedDoc } : {}),
      });
      if (draftId === null || selectedSessionId !== null) return;
      useDraftSessionsStore.getState().updateContent(draftId, {
        text,
        ...(nextImages === undefined ? {} : { images: nextImages }),
      });
    },
    [conversationKey, draftId, selectedSessionId],
  );

  // TipTap's replaceText/clear emit onTextChange; skipping park writes there keeps
  // attachmentsRef and the store aligned during hydrate and send-failure restore.
  const suppressPersistRef = useRef(false);
  /**
   * Blocks slash/@ menus after programmatic hydrate without calling setState in
   * the conversation layout effect (react-hooks/set-state-in-effect). Cleared on
   * the next user-driven doc change.
   */
  const suppressQueryMenuRef = useRef(false);

  /**
   * Conversation-keyed React state is adjusted during render so it stays inside
   * the same `act()` as the selection store update. TipTap `setContent` still
   * waits for a microtask (flushSync), but must not call setState from there.
   */
  const syncedSurfaceRef = useRef<string | null>(null);
  if (syncedSurfaceRef.current !== conversationKey) {
    const previousKey = syncedSurfaceRef.current;
    // Park the surface we are leaving before adopting the new key. The editor
    // still shows the old session until hydrate's microtask runs; without this,
    // onTextChange would write that stale document onto the new session id.
    if (previousKey !== null) {
      const editor = editorRef.current;
      if (editor !== null) {
        useComposerInputStore.getState().setInput(previousKey, {
          text: editor.getText(),
          images: [...attachmentsRef.current],
          doc: editor.getJSON(),
        });
      }
    }
    suppressPersistRef.current = true;
    syncedSurfaceRef.current = conversationKey;
    const parked = useComposerInputStore.getState().byKey[conversationKey];
    const draft =
      draftId !== null && selectedSessionId === null
        ? useDraftSessionsStore
            .getState()
            .drafts.find((candidate) => candidate.id === draftId)
        : undefined;
    const text = parked !== undefined ? parked.text : (draft?.text ?? "");
    const images = parked !== undefined ? parked.images : (draft?.images ?? []);
    replaceAttachments(images);
    const nextQuery = queryStateFromText(text, text);
    setQuery((current) =>
      queryStatesEqual(current, nextQuery) ? current : nextQuery,
    );
    setPlusMenuOpen(false);
    setMenuDismissed(false);
    setAttachmentError(null);
    suppressQueryMenuRef.current = true;
    setExpandedGroups((current) => (current.size === 0 ? current : new Set()));
    setSelectedActionIndex(0);
  }

  /** Sets editor content and attachments without re-entering persistComposerInput. */
  const applyComposerContent = useCallback(
    (text: string, images: ImageAttachment[], doc?: JSONContent) => {
      suppressPersistRef.current = true;
      // Parked `@…` / `/…` suffixes must not pop the palette on session restore.
      suppressQueryMenuRef.current = true;
      try {
        replaceAttachments(images);
        if (doc !== undefined) {
          try {
            editorRef.current?.replaceDocument(doc);
          } catch {
            // A stale or hand-edited parked doc (schema drift) must not break
            // hydration: fall back to the text-only markdown restore so the
            // composer still comes up with the typed message.
            editorRef.current?.replaceText(text);
          }
        } else if (text.length === 0) {
          editorRef.current?.clear();
        } else {
          editorRef.current?.replaceText(text);
        }
      } finally {
        suppressPersistRef.current = false;
      }
    },
    [replaceAttachments],
  );

  const conversationKeyRef = useRef(conversationKey);
  conversationKeyRef.current = conversationKey;

  const hydratedConversationKey = useRef<string | null>(null);
  const pendingHydrateKeyRef = useRef<string | null>(null);
  const hydrateGenerationRef = useRef(0);
  // Passive effect + microtask: TipTap setContent uses flushSync; calling it inside
  // useLayoutEffect nests flushSync in React's lifecycle and fails the stderr gate.
  useEffect(() => {
    if (hydratedConversationKey.current === conversationKey) return;
    if (pendingHydrateKeyRef.current === conversationKey) return;

    const previousKey = hydratedConversationKey.current;
    const key = conversationKey;
    const parked = useComposerInputStore.getState().byKey[key];
    const draft =
      draftId !== null && selectedSessionId === null
        ? useDraftSessionsStore
            .getState()
            .drafts.find((candidate) => candidate.id === draftId)
        : undefined;
    const text =
      parked !== undefined
        ? parked.text
        : draft !== undefined
          ? (draft.text ?? "")
          : "";
    const images =
      parked !== undefined
        ? parked.images
        : draft !== undefined
          ? (draft.images ?? [])
          : [];
    const doc = parked?.doc;
    const emptyPayload =
      text.length === 0 && images.length === 0 && doc === undefined;
    // First mount onto an empty conversation: the editor is already blank.
    // Scheduling a deferred clear races pastes/typing that land before the
    // microtask and would wipe them.
    if (emptyPayload && previousKey === null) {
      hydratedConversationKey.current = key;
      suppressPersistRef.current = false;
      return;
    }

    const generation = (hydrateGenerationRef.current += 1);
    pendingHydrateKeyRef.current = key;
    queueMicrotask(() => {
      try {
        if (pendingHydrateKeyRef.current === key) {
          pendingHydrateKeyRef.current = null;
        }
        if (hydrateGenerationRef.current !== generation) return;
        if (conversationKeyRef.current !== key) return;
        hydratedConversationKey.current = key;
        applyComposerContent(text, images, doc);
      } finally {
        if (conversationKeyRef.current === key) {
          suppressPersistRef.current = false;
        }
      }
    });
  }, [applyComposerContent, conversationKey, draftId, selectedSessionId]);

  // Quotes insert here directly. A pending store that the composer re-read on
  // session switch / Strict Mode replayed chips the user had already deleted.
  useEffect(() => {
    let active = true;
    const unbind = bindFileContextDelivery(conversationKey, (selections) => {
      if (!active) return;
      const editor = editorRef.current;
      // The child editor can remount while this Composer stays mounted, so the
      // handle is not guaranteed here. Either way the quote has nowhere to go
      // and is not re-queued (that is what replayed deleted chips before), so
      // the user has to be told rather than left staring at an unchanged box.
      if (editor === null) {
        setAttachmentError(t("chat.fileContext.injectFailed"));
        return;
      }
      try {
        editor.insertFileChips(selections);
        editor.focus({ at: "end" });
      } catch {
        setAttachmentError(t("chat.fileContext.injectFailed"));
      }
    });
    return () => {
      active = false;
      unbind();
    };
  }, [bindFileContextDelivery, conversationKey, t]);
  const slashQuery = query.slashQuery;
  const atQuery = query.atQuery;
  const fileMentionEnabled =
    !plusMenuOpen && !menuDismissed && !disabled && !isResponding;
  const fileMentions = useComposerFileMentions({
    taskId,
    projectId,
    atQuery,
    enabled: fileMentionEnabled,
  });
  const fileMentionActive = fileMentions.active;

  const allActions = useMemo(
    () =>
      buildComposerActions({
        skills,
        commands: availableCommands,
        roles,
        plugins: composerPlugins,
        translatePluginSummary: (summaryKey) => t(summaryKey),
        includeAttachments: true,
        attachmentLabel: t("chat.actionMenu.addImages"),
        attachmentDescription: t("chat.actionMenu.addImagesDescription"),
      }),
    [availableCommands, composerPlugins, roles, skills, t],
  );
  const filteredActions = useMemo(() => {
    if (plusMenuOpen) return filterComposerActions(allActions, "");
    if (atQuery !== null) {
      // Loading/error/debounce clears paths so the menu never offers stale hits.
      return buildComposerFileActions(fileMentions.entries);
    }
    // Slash is for skills and commands only; plugins are reached through the "+" menu.
    return filterComposerActions(
      allActions.filter((action) => action.group !== "plugins"),
      slashQuery ?? "",
    );
  }, [allActions, atQuery, fileMentions.entries, plusMenuOpen, slashQuery]);
  const visibleActions = useMemo(
    () => visibleComposerActions(filteredActions, expandedGroups),
    [expandedGroups, filteredActions],
  );
  const fileMenuStatus = fileMentionMenuStatus(fileMentions.status);
  const fileMenuStatusMessageKey = fileMentionStatusMessageKey(
    fileMentions.status,
    fileMentions.debouncedQuery,
  );
  const fileMenuStatusMessage =
    fileMenuStatusMessageKey === undefined
      ? undefined
      : t(fileMenuStatusMessageKey);
  const showActionMenu =
    (plusMenuOpen ||
      (slashQuery !== null &&
        !menuDismissed &&
        !suppressQueryMenuRef.current) ||
      (atQuery !== null && !menuDismissed && !suppressQueryMenuRef.current)) &&
    !disabled &&
    !isResponding &&
    (visibleActions.length > 0 || fileMentionActive);

  const hasText = !query.isBlank;
  const canSend =
    (hasText || attachments.length > 0) && !isResponding && !disabled;

  const submit = () => {
    if (isResponding || disabled) return;
    const text = (editorRef.current?.getText() ?? "").trim();
    const sentAttachments = attachments;
    const sentDoc = editorRef.current?.getJSON();
    const sentImages =
      sentAttachments.length === 0
        ? undefined
        : sentAttachments.map((attachment) => attachment.content);
    // Capture the surface this send left so a reject after navigation cannot
    // paint the abandoned message onto a different conversation's composer.
    const sendConversationKey = conversationKey;
    const sendDraftId = draftId;
    const sendSessionId = selectedSessionId;
    const submitGeneration = (submitGenerationRef.current += 1);
    // Async wrapping turns a synchronous callback throw into the same rejected
    // promise path used by transport failures, so restoration always runs.
    const sendResult = (async () => {
      if (sentImages === undefined) await onSend(text);
      else await onSend(text, sentImages);
    })();
    applyComposerContent("", []);
    setQuery(EMPTY_COMPOSER_QUERY);
    setAttachmentError(null);
    // Drop the conversation-keyed park so a later return cannot resurrect the
    // message that was just sent. Draft-store text is left alone so the muted
    // sidebar title survives until attach replaces the row.
    useComposerInputStore.getState().clear(conversationKey);
    closeActionMenu();
    // If the send rejects while still on this surface, put the message back.
    // Abandoned sends (Stop / navigated away) already repark stores — restore the
    // reused composer UI only when we never left, so a later surface is not
    // contaminated. Hard failures restore only on the send surface, the
    // recovered draft, or the warm session that first-send adopted — never an
    // unrelated chat the user opened mid-attach. A newer submit supersedes
    // this catch entirely (Stop → retype → send).
    //
    // Use async/await (not Promise.resolve().then) so reject handlers attach
    // directly to the send promise; tests can await the same promise inside
    // act() and CI's stderr gate will not see stray setState warnings.
    void (async () => {
      try {
        await sendResult;
        clearComposerSendAdoption(sendConversationKey);
      } catch (error: unknown) {
        try {
          if (submitGeneration !== submitGenerationRef.current) return;
          const selection = useWorkspaceSelectionStore.getState().selection;
          const onSendSurface =
            conversationKeyFor(selection) === sendConversationKey &&
            selection.draftId === sendDraftId &&
            selection.sessionId === sendSessionId;
          let adoptedSession: string | undefined;
          if (error instanceof DraftSendAbandonedError) {
            // Workspace already reparked text/images onto the draft; attach the
            // TipTap tree so / and directory chips survive when the user returns
            // (including after they navigated away mid-handshake).
            if (sendDraftId !== null && sentDoc !== undefined) {
              const draftKey = `draft:${sendDraftId}`;
              const parked = useComposerInputStore.getState().byKey[draftKey];
              useComposerInputStore.getState().setInput(draftKey, {
                text: parked?.text ?? text,
                images: parked?.images ?? sentAttachments,
                doc: sentDoc,
              });
            } else if (sentDoc !== undefined) {
              persistComposerInput(text, sentAttachments, sentDoc);
            }
            if (!onSendSurface) return;
          } else {
            adoptedSession = composerSendAdoptedSession(sendConversationKey);
            const onSendDraft =
              sendDraftId !== null && selection.draftId === sendDraftId;
            const onAdoptedSession =
              adoptedSession !== undefined &&
              selection.sessionId === adoptedSession;
            if (!onSendSurface && !onSendDraft && !onAdoptedSession) return;
          }
          if (
            adoptedSession !== undefined &&
            selection.sessionId === adoptedSession
          ) {
            useComposerInputStore.getState().setInput(adoptedSession, {
              text,
              images: sentAttachments,
              ...(sentDoc !== undefined ? { doc: sentDoc } : {}),
            });
          } else {
            persistComposerInput(text, sentAttachments, sentDoc);
          }
          applyComposerContent(text, sentAttachments, sentDoc);
          setQuery(queryStateFromText(text, text));
        } finally {
          clearComposerSendAdoption(sendConversationKey);
        }
      }
    })();
  };

  /** Inserts a skill, command, or role mention so the token stays distinct from body text. */
  const insertPromptToken = (kind: PromptTokenKind, name: string) => {
    editorRef.current?.insertPromptToken(kind, name);
    closeActionMenu();
    requestAnimationFrame(() => editorRef.current?.focus());
  };

  /** Adds a plugin to this message's applied set (reached from the "+" menu). */
  const applyPlugin = (plugin: PluginEntry) => {
    addSelectedPlugin(conversationKey, plugin.id);
    closeActionMenu();
    requestAnimationFrame(() => editorRef.current?.focus());
  };

  /** Inserts a workspace path chip (file or folder) and clears the `@…` token. */
  const insertFileMention = (path: string, entryKind: "file" | "directory") => {
    editorRef.current?.removeAtToken();
    editorRef.current?.insertFileChips([{ path, kind: entryKind }]);
    closeActionMenu();
    requestAnimationFrame(() => editorRef.current?.focus({ at: "keep" }));
  };

  /** Executes the selected palette action through its existing product data path. */
  const selectAction = (action: ComposerAction) => {
    switch (action.group) {
      case "files":
        insertFileMention(action.path, action.entryKind);
        return;
      case "skills":
        insertPromptToken("skill", action.skill.name);
        return;
      case "commands":
        insertPromptToken("command", action.command.name);
        return;
      case "roles":
        insertPromptToken("role", action.role.name);
        return;
      case "plugins":
        applyPlugin(action.plugin);
        return;
      case "actions":
        closeActionMenu();
        fileInputRef.current?.click();
    }
  };

  /** Closes both menu triggers and restores the collapsed section state. */
  function closeActionMenu() {
    setPlusMenuOpen(false);
    setMenuDismissed(true);
    setExpandedGroups(new Set());
  }

  /** Converts selected files into ACP images while enforcing a bounded prompt payload. */
  const addImages = async (files: Iterable<File> | null) => {
    if (files === null) return;
    const selectedFiles = [...files];
    if (selectedFiles.length === 0) return;
    const totalBytes =
      attachmentsRef.current.reduce(
        (sum, attachment) => sum + attachment.size,
        0,
      ) + selectedFiles.reduce((sum, file) => sum + file.size, 0);
    if (selectedFiles.some((file) => !ACCEPTED_IMAGE_TYPES.has(file.type))) {
      setAttachmentError(t("chat.attachments.unsupported"));
      return;
    }
    if (
      selectedFiles.some((file) => file.size > MAX_IMAGE_BYTES) ||
      totalBytes > MAX_TOTAL_IMAGE_BYTES
    ) {
      setAttachmentError(t("chat.attachments.tooLarge"));
      return;
    }
    const next = await Promise.all(selectedFiles.map(readImageAttachment));
    const combined = [...attachmentsRef.current, ...next];
    replaceAttachments(combined);
    persistComposerInput(editorRef.current?.getText() ?? "", combined);
    setAttachmentError(null);
  };

  /** Adds clipboard files through the same validation path as the attachment picker. */
  const handlePasteFiles = (files: File[]) => {
    if (files.length === 0) return;
    void addImages(files).catch(() =>
      setAttachmentError(t("chat.attachments.readFailed")),
    );
  };

  const handleMenuKeyDown = (event: KeyboardEvent): boolean => {
    if (!showActionMenu) return false;
    if (visibleActions.length === 0) {
      if (event.key === "Escape") {
        event.preventDefault();
        closeActionMenu();
        return true;
      }
      // Keep Enter/Tab from sending while the @ file palette is open without hits.
      if (
        fileMentionActive &&
        (event.key === "Enter" || event.key === "Tab") &&
        !event.isComposing
      ) {
        event.preventDefault();
        return true;
      }
      return false;
    }
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      const direction = event.key === "ArrowDown" ? 1 : -1;
      actionHighlightSourceRef.current = "keyboard";
      setSelectedActionIndex(
        (current) =>
          (current + direction + visibleActions.length) % visibleActions.length,
      );
      return true;
    }
    if (event.key === "Escape") {
      event.preventDefault();
      closeActionMenu();
      return true;
    }
    if ((event.key === "Enter" || event.key === "Tab") && !event.isComposing) {
      event.preventDefault();
      // Debounce / in-flight: keep highlighting but do not commit a stale path.
      if (fileMentionActive && fileMentions.selectionLocked) {
        return true;
      }
      const action = visibleActions[selectedActionIndex];
      if (action !== undefined) selectAction(action);
      return true;
    }
    return false;
  };

  const handleDocChange = () => {
    // Programmatic hydrate/clear must not reopen @ / menus from a parked `@…`
    // or `/…` suffix at the caret.
    if (suppressPersistRef.current) return;
    suppressQueryMenuRef.current = false;
    setPlusMenuOpen(false);
    setMenuDismissed(false);
    setExpandedGroups((current) => (current.size === 0 ? current : new Set()));
    actionHighlightSourceRef.current = "keyboard";
    setSelectedActionIndex(0);
  };

  // Render-phase highlight sync (avoid setState-in-effect): reset when the
  // settled @ query changes, and clamp when the visible list shrinks.
  const [highlightQueryKey, setHighlightQueryKey] = useState(
    fileMentions.debouncedQuery,
  );
  if (fileMentionActive && fileMentions.debouncedQuery !== highlightQueryKey) {
    setHighlightQueryKey(fileMentions.debouncedQuery);
    actionHighlightSourceRef.current = "keyboard";
    setSelectedActionIndex(0);
  }
  if (
    visibleActions.length > 0 &&
    selectedActionIndex >= visibleActions.length
  ) {
    setSelectedActionIndex(visibleActions.length - 1);
  }

  // Hide the model picker only when the footer cannot fit both control groups.
  // The stored width lets the check continue to work after the picker is hidden.
  useLayoutEffect(() => {
    const footer = footerRef.current;
    const leftControls = leftControlsRef.current;
    const rightControls = rightControlsRef.current;
    if (!footer || !leftControls || !rightControls) return;

    const updateModelVisibility = () => {
      const footerWidth = footer.clientWidth;
      const rightControlsWidth = rightControls.getBoundingClientRect().width;
      if (footerWidth === 0 || rightControlsWidth === 0) return;

      if (showModelSelector) {
        fullRightControlsWidthRef.current = rightControlsWidth;
      }

      const requiredRightWidth = showModelSelector
        ? rightControlsWidth
        : fullRightControlsWidthRef.current;
      if (requiredRightWidth === null) return;

      const leftControlsRect = leftControls.getBoundingClientRect();
      const rightControlsRect = rightControls.getBoundingClientRect();
      const footerGap =
        Number.parseFloat(getComputedStyle(footer).columnGap) || 0;
      const leftControlsWidth = Math.max(
        leftControls.scrollWidth,
        leftControlsRect.width,
      );
      const doesOverflow =
        leftControls.scrollWidth > leftControls.clientWidth + 1 ||
        leftControlsRect.right > rightControlsRect.left + 1;
      const doesNotFit =
        leftControlsWidth + footerGap + requiredRightWidth > footerWidth + 1;
      const nextVisibility = !doesOverflow && !doesNotFit;

      setShowModelSelector((current) =>
        current === nextVisibility ? current : nextVisibility,
      );
    };

    updateModelVisibility();
    if (typeof ResizeObserver === "undefined") return;

    const observer = new ResizeObserver(updateModelVisibility);
    observer.observe(footer);
    observer.observe(leftControls);
    observer.observe(rightControls);
    return () => observer.disconnect();
  }, [showModelSelector]);

  useEffect(() => {
    if (!showActionMenu || visibleActions.length === 0) return;
    // Pointer highlight already follows the cursor; scrolling that row into view
    // fights the wheel and snaps the list back toward the active item.
    if (actionHighlightSourceRef.current !== "keyboard") return;
    const safeIndex = Math.min(selectedActionIndex, visibleActions.length - 1);
    actionOptionRefs.current[safeIndex]?.scrollIntoView?.({ block: "nearest" });
  }, [selectedActionIndex, showActionMenu, visibleActions.length]);

  useEffect(() => {
    if (!showActionMenu) return;
    const dismissOutside = (event: PointerEvent) => {
      if (!composerRef.current?.contains(event.target as Node))
        closeActionMenu();
    };
    document.addEventListener("pointerdown", dismissOutside);
    return () => document.removeEventListener("pointerdown", dismissOutside);
  }, [showActionMenu]);

  return (
    <div
      ref={composerRef}
      data-slot="composer"
      className="relative flex flex-col rounded-xl border border-border bg-card shadow-[0_1px_3px_rgba(0,0,0,0.06),0_8px_24px_rgba(0,0,0,0.04)] transition-[border-color,box-shadow] duration-200 hover:border-foreground/20 hover:shadow-[0_2px_4px_rgba(0,0,0,0.06),0_10px_28px_rgba(0,0,0,0.06)] focus-within:border-foreground/30 focus-within:shadow-[0_2px_4px_rgba(0,0,0,0.07),0_12px_32px_rgba(0,0,0,0.07)] focus-within:ring-2 focus-within:ring-ring/25 dark:shadow-[0_1px_3px_rgba(0,0,0,0.28),0_10px_28px_rgba(0,0,0,0.18)]"
    >
      {showActionMenu && (
        <ComposerActionMenu
          id={actionMenuId}
          actions={filteredActions}
          activeIndex={selectedActionIndex}
          expandedGroups={expandedGroups}
          optionRefs={actionOptionRefs}
          status={fileMentionActive ? fileMenuStatus : "ready"}
          statusMessage={fileMentionActive ? fileMenuStatusMessage : undefined}
          truncated={fileMentions.truncated}
          filesPalette={fileMentionActive}
          selectionLocked={fileMentionActive && fileMentions.selectionLocked}
          onActiveIndexChange={(index) => {
            actionHighlightSourceRef.current = "pointer";
            setSelectedActionIndex(index);
          }}
          onToggleGroup={(group) => {
            setExpandedGroups((current) => {
              const next = new Set(current);
              if (next.has(group)) next.delete(group);
              else next.add(group);
              return next;
            });
            setSelectedActionIndex(0);
          }}
          onSelect={selectAction}
        />
      )}
      <div className="flex flex-col p-2">
        {attachments.length > 0 && (
          <div
            className="flex gap-2 overflow-x-auto px-2 pb-2 pt-1"
            aria-label={t("chat.attachments.selected")}
          >
            {attachments.map((attachment) => (
              <figure
                key={attachment.id}
                className="group/attachment relative size-16 shrink-0 overflow-hidden rounded-md border border-border bg-muted"
              >
                <button
                  type="button"
                  onClick={() => setPreviewedAttachment(attachment)}
                  aria-label={t("chat.content.previewImage", {
                    name: attachment.name,
                  })}
                  className="size-full cursor-zoom-in outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
                >
                  <img
                    src={`data:${attachment.content.mimeType};base64,${attachment.content.data}`}
                    alt={attachment.name}
                    className="size-full object-cover"
                  />
                </button>
                <button
                  type="button"
                  onClick={() => {
                    const next = attachmentsRef.current.filter(
                      (item) => item.id !== attachment.id,
                    );
                    replaceAttachments(next);
                    persistComposerInput(
                      editorRef.current?.getText() ?? "",
                      next,
                    );
                  }}
                  aria-label={t("chat.attachments.remove", {
                    name: attachment.name,
                  })}
                  className="absolute right-1 top-1 flex size-6 cursor-pointer items-center justify-center rounded-md bg-black/70 text-white opacity-0 outline-none transition-opacity duration-150 hover:bg-black focus-visible:opacity-100 focus-visible:ring-2 focus-visible:ring-white group-hover/attachment:opacity-100"
                >
                  <IconX className="size-3.5" />
                </button>
              </figure>
            ))}
          </div>
        )}
        {attachmentError && (
          <p role="alert" className="px-2 pb-1 text-[11px] text-destructive">
            {attachmentError}
          </p>
        )}
        <ComposerEditor
          ref={editorRef}
          autoFocus={autoFocus}
          placeholder={placeholder ?? t("chat.placeholder")}
          disabled={disabled}
          ariaLabel={t("chat.messageLabel")}
          ariaAutoComplete="list"
          ariaHasPopup="listbox"
          ariaExpanded={showActionMenu}
          ariaControls={showActionMenu ? actionMenuId : undefined}
          ariaActivedescendant={
            showActionMenu
              ? `${actionMenuId}-option-${selectedActionIndex}`
              : undefined
          }
          onSubmit={submit}
          onQueryChange={setQuery}
          onDocChange={handleDocChange}
          onTextChange={(text) => {
            if (suppressPersistRef.current) return;
            persistComposerInput(text);
          }}
          onPasteFiles={handlePasteFiles}
          onMenuKeyDown={handleMenuKeyDown}
        />
        <div
          ref={footerRef}
          className="flex min-h-8 items-center justify-between gap-2 pt-0.5"
        >
          <div
            ref={leftControlsRef}
            className="flex min-w-0 items-center gap-1"
          >
            <input
              ref={fileInputRef}
              type="file"
              accept={[...ACCEPTED_IMAGE_TYPES].join(",")}
              multiple
              className="sr-only"
              onChange={(event) => {
                void addImages(event.target.files).catch(() =>
                  setAttachmentError(t("chat.attachments.readFailed")),
                );
                event.target.value = "";
              }}
            />
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              disabled={disabled || isResponding}
              aria-label={t("chat.actionMenu.open")}
              aria-haspopup="listbox"
              aria-expanded={showActionMenu && plusMenuOpen}
              aria-controls={
                showActionMenu && plusMenuOpen ? actionMenuId : undefined
              }
              onClick={() => {
                setPlusMenuOpen((current) => !current);
                setMenuDismissed(false);
                setExpandedGroups(new Set());
                setSelectedActionIndex(0);
              }}
              className="rounded-full text-muted-foreground"
            >
              <IconPlus
                className={`size-4 transition-transform duration-150 motion-reduce:transition-none ${plusMenuOpen ? "rotate-45" : ""}`}
              />
            </Button>
            {attachments.length > 0 && (
              <IconPhoto
                className="size-3.5 text-sky-600 dark:text-sky-400"
                aria-hidden="true"
              />
            )}
            <PermissionSelector disabled={disabled} />
            {selectedPlugins.length > 0 && (
              <SelectedPluginsButton
                selected={selectedPlugins}
                disabled={disabled}
                onRemove={(plugin) =>
                  removeSelectedPlugin(conversationKey, plugin.id)
                }
              />
            )}
          </div>
          <div
            ref={rightControlsRef}
            className="flex shrink-0 items-center gap-2"
          >
            {showModelSelector && (
              <ModelSelector
                disabled={modelSelectorDisabled}
                sessionId={modelSelectorSessionId}
              />
            )}
            <Button
              size="icon"
              // A live turn always stops on click, whether it is still starting up
              // (spinner) or already streaming (stop icon); only idle sends.
              aria-label={
                isResponding
                  ? isStreaming
                    ? t("common.stop")
                    : t("chat.starting")
                  : t("chat.send")
              }
              disabled={isResponding ? onStop === undefined : !canSend}
              onClick={isResponding ? onStop : submit}
              className="size-8 rounded-full bg-foreground text-background shadow-sm transition-[background-color,color,box-shadow] duration-200 hover:bg-foreground/85 hover:shadow-md disabled:bg-muted disabled:text-muted-foreground disabled:shadow-none"
            >
              {isResponding ? (
                isStreaming ? (
                  <IconPlayerStop className="size-[18px]" />
                ) : (
                  <IconLoader2 className="size-[18px] animate-spin" />
                )
              ) : (
                <IconArrowUp className="size-[18px]" />
              )}
            </Button>
          </div>
        </div>
      </div>
      {previewedAttachment && (
        <ImagePreviewDialog
          open
          src={`data:${previewedAttachment.content.mimeType};base64,${previewedAttachment.content.data}`}
          name={previewedAttachment.name}
          onOpenChange={(open) => !open && setPreviewedAttachment(null)}
        />
      )}
    </div>
  );
}

/** Reads one browser image into the base64 payload required by ACP. */
function readImageAttachment(file: File): Promise<ImageAttachment> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () =>
      reject(reader.error ?? new Error("failed to read image"));
    reader.onload = () => {
      const result = reader.result;
      if (typeof result !== "string") {
        reject(new Error("failed to read image"));
        return;
      }
      const separator = result.indexOf(",");
      if (separator === -1) {
        reject(new Error("invalid image data"));
        return;
      }
      resolve({
        id: crypto.randomUUID(),
        name: file.name,
        size: file.size,
        content: {
          data: result.slice(separator + 1),
          mimeType: file.type,
          uri: file.name,
        },
      });
    };
    reader.readAsDataURL(file);
  });
}
