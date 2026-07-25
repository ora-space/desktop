import { useEffect, useId, useMemo, useRef, useState } from "react";
import type { ClipboardEvent, KeyboardEvent } from "react";
import { IconArrowUp, IconLoader2, IconPhoto, IconPlayerStop, IconPlus, IconX } from "@tabler/icons-react";
import { Button, Textarea } from "@ora/ui";
import type { acp, Skill } from "@ora/contracts";
import { useTranslation } from "react-i18next";
import { ModelSelector } from "./model-selector";
import { PermissionSelector } from "./permission-selector";
import { WorkflowToggle } from "../workflow/workflow-toggle";
import {
  ComposerActionMenu,
} from "./composer-action-menu";
import { ImagePreviewDialog } from "./image-preview-dialog";
import {
  buildComposerActions,
  filterComposerActions,
  visibleComposerActions,
  type ComposerAction,
  type ComposerActionGroup,
} from "./composer-actions";

interface ComposerProps {
  onSend: (text: string, images?: acp.ImageContent[]) => void;
  /**
   * Invoked when Enter (or send) is pressed with an empty input. Used in Spec mode
   * to run the highlighted stage directly; absent when there is nothing to launch.
  */
  onEmptySubmit?: () => void;
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
  placeholder?: string;
  autoFocus?: boolean;
  skills?: Skill[];
  availableCommands?: acp.AvailableCommand[];
  modes?: acp.SessionModeState | null;
  onModeChange?: (modeId: acp.SessionModeId) => Promise<void>;
}

interface ImageAttachment {
  id: string;
  name: string;
  size: number;
  content: acp.ImageContent;
}

const ACCEPTED_IMAGE_TYPES = new Set(["image/avif", "image/bmp", "image/gif", "image/jpeg", "image/png", "image/webp"]);
const MAX_IMAGE_BYTES = 5 * 1024 * 1024;
const MAX_TOTAL_IMAGE_BYTES = 10 * 1024 * 1024;

/**
 * The chat composer: a rounded input shell wrapping the @ora/ui Textarea with
 * an inline send button. Enter sends, Shift+Enter inserts a newline, and the
 * textarea auto-grows up to a max height.
 */
export function Composer({
  onSend,
  onEmptySubmit,
  onStop,
  isResponding,
  isStreaming = false,
  disabled = false,
  placeholder,
  autoFocus = false,
  skills = [],
  availableCommands = [],
  modes = null,
  onModeChange,
}: ComposerProps) {
  const { t } = useTranslation();
  const [value, setValue] = useState("");
  const [selectedActionIndex, setSelectedActionIndex] = useState(0);
  const [menuDismissed, setMenuDismissed] = useState(false);
  const [plusMenuOpen, setPlusMenuOpen] = useState(false);
  const [expandedGroups, setExpandedGroups] = useState<ReadonlySet<ComposerActionGroup>>(new Set());
  const [attachments, setAttachments] = useState<ImageAttachment[]>([]);
  const [attachmentError, setAttachmentError] = useState<string | null>(null);
  const [previewedAttachment, setPreviewedAttachment] = useState<ImageAttachment | null>(null);
  const composerRef = useRef<HTMLDivElement>(null);
  const textAreaRef = useRef<HTMLTextAreaElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const actionOptionRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const actionMenuId = useId();
  const slashQuery = value.match(/^\/([^\s]*)$/)?.[1] ?? null;
  const allActions = useMemo(() => buildComposerActions({
    skills,
    commands: availableCommands,
    modes: onModeChange === undefined ? null : modes,
    includeAttachments: true,
    attachmentLabel: t("chat.actionMenu.addImages"),
    attachmentDescription: t("chat.actionMenu.addImagesDescription"),
  }), [availableCommands, modes, onModeChange, skills, t]);
  const filteredActions = useMemo(
    () => filterComposerActions(allActions, plusMenuOpen ? "" : slashQuery ?? ""),
    [allActions, plusMenuOpen, slashQuery],
  );
  const visibleActions = useMemo(
    () => visibleComposerActions(filteredActions, expandedGroups),
    [expandedGroups, filteredActions],
  );
  const showActionMenu = visibleActions.length > 0
    && (plusMenuOpen || (slashQuery !== null && !menuDismissed))
    && !disabled
    && !isResponding;

  const hasText = value.trim().length > 0;
  // With an empty input the send affordance still fires when there is a stage to
  // launch, so pressing Enter runs the highlighted step.
  const canSend = (hasText || attachments.length > 0 || onEmptySubmit !== undefined)
    && !isResponding
    && !disabled;

  const submit = () => {
    if (isResponding || disabled) return;
    const text = value.trim();
    if (text === "" && attachments.length === 0) {
      onEmptySubmit?.();
      return;
    }
    if (attachments.length === 0) onSend(text);
    else onSend(text, attachments.map((attachment) => attachment.content));
    setValue("");
    setAttachments([]);
    setAttachmentError(null);
    closeActionMenu();
  };

  /** Inserts a skill or command token for review while keeping arguments under user control. */
  const insertPromptToken = (inserted: string) => {
    setValue(inserted);
    closeActionMenu();
    requestAnimationFrame(() => {
      textAreaRef.current?.focus();
      textAreaRef.current?.setSelectionRange(inserted.length, inserted.length);
    });
  };

  /** Executes the selected palette action through its existing product data path. */
  const selectAction = (action: ComposerAction) => {
    switch (action.group) {
      case "skills":
        insertPromptToken(`$${action.skill.name} `);
        return;
      case "commands":
        insertPromptToken(`/${action.command.name} `);
        return;
      case "modes":
        closeActionMenu();
        void onModeChange?.(action.mode.id).catch(() => undefined);
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
    const totalBytes = attachments.reduce((sum, attachment) => sum + attachment.size, 0)
      + selectedFiles.reduce((sum, file) => sum + file.size, 0);
    if (selectedFiles.some((file) => !ACCEPTED_IMAGE_TYPES.has(file.type))) {
      setAttachmentError(t("chat.attachments.unsupported"));
      return;
    }
    if (selectedFiles.some((file) => file.size > MAX_IMAGE_BYTES) || totalBytes > MAX_TOTAL_IMAGE_BYTES) {
      setAttachmentError(t("chat.attachments.tooLarge"));
      return;
    }
    const next = await Promise.all(selectedFiles.map(readImageAttachment));
    setAttachments((current) => [...current, ...next]);
    setAttachmentError(null);
  };

  /** Adds clipboard files through the same validation path as the attachment picker. */
  const handlePaste = (event: ClipboardEvent<HTMLTextAreaElement>) => {
    const files = [...event.clipboardData.files];
    if (files.length === 0) return;
    event.preventDefault();
    void addImages(files).catch(() => setAttachmentError(t("chat.attachments.readFailed")));
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (showActionMenu) {
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        const direction = event.key === "ArrowDown" ? 1 : -1;
        setSelectedActionIndex((current) =>
          (current + direction + visibleActions.length) % visibleActions.length,
        );
        return;
      }
      if (event.key === "Escape") {
        event.preventDefault();
        closeActionMenu();
        return;
      }
      if ((event.key === "Enter" || event.key === "Tab") && !event.nativeEvent.isComposing) {
        event.preventDefault();
        const action = visibleActions[selectedActionIndex];
        if (action !== undefined) selectAction(action);
        return;
      }
    }
    if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
      event.preventDefault();
      submit();
    }
  };

  // Auto-grow the textarea to fit its content, capped at a comfortable max.
  useEffect(() => {
    const el = textAreaRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 200)}px`;
  }, [value]);

  useEffect(() => {
    if (!showActionMenu) return;
    const safeIndex = Math.min(selectedActionIndex, visibleActions.length - 1);
    actionOptionRefs.current[safeIndex]?.scrollIntoView?.({ block: "nearest" });
  }, [selectedActionIndex, showActionMenu, visibleActions.length]);

  useEffect(() => {
    if (!showActionMenu) return;
    const dismissOutside = (event: PointerEvent) => {
      if (!composerRef.current?.contains(event.target as Node)) closeActionMenu();
    };
    document.addEventListener("pointerdown", dismissOutside);
    return () => document.removeEventListener("pointerdown", dismissOutside);
  }, [showActionMenu]);

  return (
    <div ref={composerRef} data-slot="composer" className="relative flex flex-col rounded-xl border border-border bg-card shadow-[0_1px_3px_rgba(0,0,0,0.06),0_8px_24px_rgba(0,0,0,0.04)] transition-[border-color,box-shadow] duration-200 hover:border-foreground/20 hover:shadow-[0_2px_4px_rgba(0,0,0,0.06),0_10px_28px_rgba(0,0,0,0.06)] focus-within:border-foreground/30 focus-within:shadow-[0_2px_4px_rgba(0,0,0,0.07),0_12px_32px_rgba(0,0,0,0.07)] focus-within:ring-2 focus-within:ring-ring/25 dark:shadow-[0_1px_3px_rgba(0,0,0,0.28),0_10px_28px_rgba(0,0,0,0.18)]">
      {showActionMenu && (
        <ComposerActionMenu
          id={actionMenuId}
          actions={filteredActions}
          activeIndex={selectedActionIndex}
          currentModeId={modes?.currentModeId}
          expandedGroups={expandedGroups}
          optionRefs={actionOptionRefs}
          onActiveIndexChange={setSelectedActionIndex}
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
          <div className="flex gap-2 overflow-x-auto px-2 pb-2 pt-1" aria-label={t("chat.attachments.selected")}>
            {attachments.map((attachment) => (
              <figure key={attachment.id} className="group/attachment relative size-16 shrink-0 overflow-hidden rounded-md border border-border bg-muted">
                <button
                  type="button"
                  onClick={() => setPreviewedAttachment(attachment)}
                  aria-label={t("chat.content.previewImage", { name: attachment.name })}
                  className="size-full cursor-zoom-in outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
                >
                  <img src={`data:${attachment.content.mimeType};base64,${attachment.content.data}`} alt={attachment.name} className="size-full object-cover" />
                </button>
                <button
                  type="button"
                  onClick={() => setAttachments((current) => current.filter((item) => item.id !== attachment.id))}
                  aria-label={t("chat.attachments.remove", { name: attachment.name })}
                  className="absolute right-1 top-1 flex size-6 cursor-pointer items-center justify-center rounded-md bg-black/70 text-white opacity-0 outline-none transition-opacity duration-150 hover:bg-black focus-visible:opacity-100 focus-visible:ring-2 focus-visible:ring-white group-hover/attachment:opacity-100"
                >
                  <IconX className="size-3.5" />
                </button>
              </figure>
            ))}
          </div>
        )}
        {attachmentError && <p role="alert" className="px-2 pb-1 text-[11px] text-destructive">{attachmentError}</p>}
        <Textarea
          ref={textAreaRef}
          autoFocus={autoFocus}
          placeholder={placeholder ?? t("chat.placeholder")}
          value={value}
          disabled={disabled}
          onChange={(event) => {
            setValue(event.target.value);
            setPlusMenuOpen(false);
            setMenuDismissed(false);
            setExpandedGroups(new Set());
            setSelectedActionIndex(0);
          }}
          onKeyDown={handleKeyDown}
          onPaste={handlePaste}
          aria-label={t("chat.messageLabel")}
          aria-autocomplete="list"
          aria-haspopup="listbox"
          aria-expanded={showActionMenu}
          aria-controls={showActionMenu ? actionMenuId : undefined}
          aria-activedescendant={showActionMenu ? `${actionMenuId}-option-${selectedActionIndex}` : undefined}
          // The shell already carries the surface, so the Textarea's own disabled
          // fill would read as a grey block floating inside the card.
          className="min-h-14 max-h-[200px] resize-none rounded-none border-0 bg-transparent px-2 py-1 text-[15px] leading-6 shadow-none focus-visible:ring-0 disabled:bg-transparent"
        />
        <div className="flex min-h-8 items-center justify-between gap-2 pt-0.5">
          <div className="flex min-w-0 items-center gap-1">
            <input
              ref={fileInputRef}
              type="file"
              accept={[...ACCEPTED_IMAGE_TYPES].join(",")}
              multiple
              className="sr-only"
              onChange={(event) => {
                void addImages(event.target.files).catch(() => setAttachmentError(t("chat.attachments.readFailed")));
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
              aria-controls={showActionMenu && plusMenuOpen ? actionMenuId : undefined}
              onClick={() => {
                setPlusMenuOpen((current) => !current);
                setMenuDismissed(false);
                setExpandedGroups(new Set());
                setSelectedActionIndex(0);
              }}
              className="rounded-full text-muted-foreground"
            >
              <IconPlus className={`size-4 transition-transform duration-150 motion-reduce:transition-none ${plusMenuOpen ? "rotate-45" : ""}`} />
            </Button>
            {attachments.length > 0 && <IconPhoto className="size-3.5 text-sky-600 dark:text-sky-400" aria-hidden="true" />}
            <PermissionSelector disabled={disabled} />
            <WorkflowToggle disabled={disabled} />
          </div>
          <div className="flex shrink-0 items-center gap-2">
            <ModelSelector disabled={disabled} />
            <Button
              size="icon"
              // A live turn always stops on click, whether it is still starting up
              // (spinner) or already streaming (stop icon); only idle sends.
              aria-label={isResponding ? (isStreaming ? t("common.stop") : t("chat.starting")) : t("chat.send")}
              disabled={isResponding ? onStop === undefined : !canSend}
              onClick={isResponding ? onStop : submit}
              className="size-8 rounded-full bg-foreground text-background shadow-sm transition-[background-color,color,box-shadow] duration-200 hover:bg-foreground/85 hover:shadow-md disabled:bg-muted disabled:text-muted-foreground disabled:shadow-none"
            >
              {isResponding
                ? isStreaming
                  ? <IconPlayerStop className="size-[18px]" />
                  : <IconLoader2 className="size-[18px] animate-spin" />
                : <IconArrowUp className="size-[18px]" />}
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
    reader.onerror = () => reject(reader.error ?? new Error("failed to read image"));
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
        content: { data: result.slice(separator + 1), mimeType: file.type, uri: file.name },
      });
    };
    reader.readAsDataURL(file);
  });
}
