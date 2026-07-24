import { useEffect, useId, useMemo, useRef, useState } from "react";
import type { KeyboardEvent } from "react";
import { IconArrowUp, IconCommand, IconLoader2, IconPhoto, IconPlayerStop, IconPlus, IconX } from "@tabler/icons-react";
import { Button, Textarea } from "@ora/ui";
import type { acp } from "@ora/contracts";
import { useTranslation } from "react-i18next";
import { ModelSelector } from "./model-selector";
import { PermissionSelector } from "./permission-selector";
import { WorkflowToggle } from "../workflow/workflow-toggle";
import { ModeSelector } from "./mode-selector";

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
  availableCommands = [],
  modes = null,
  onModeChange,
}: ComposerProps) {
  const { t } = useTranslation();
  const [value, setValue] = useState("");
  const [selectedCommandIndex, setSelectedCommandIndex] = useState(0);
  const [commandsDismissed, setCommandsDismissed] = useState(false);
  const [attachments, setAttachments] = useState<ImageAttachment[]>([]);
  const [attachmentError, setAttachmentError] = useState<string | null>(null);
  const textAreaRef = useRef<HTMLTextAreaElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const commandOptionRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const commandListId = useId();
  const commandQuery = value.match(/^\/([^\s]*)$/)?.[1].toLocaleLowerCase();
  const commandCandidates = useMemo(() => {
    if (commandQuery === undefined) return [];
    return availableCommands.filter((command) =>
      command.name.toLocaleLowerCase().includes(commandQuery)
      || command.description.toLocaleLowerCase().includes(commandQuery),
    );
  }, [availableCommands, commandQuery]);
  const showCommands = commandCandidates.length > 0
    && !commandsDismissed
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
    setCommandsDismissed(false);
  };

  /** Inserts a selected command for review and leaves arguments in the user's control. */
  const selectCommand = (command: acp.AvailableCommand) => {
    const inserted = `/${command.name} `;
    setValue(inserted);
    setCommandsDismissed(true);
    requestAnimationFrame(() => {
      textAreaRef.current?.focus();
      textAreaRef.current?.setSelectionRange(inserted.length, inserted.length);
    });
  };

  /** Converts selected files into ACP images while enforcing a bounded prompt payload. */
  const addImages = async (files: FileList | null) => {
    if (files === null || files.length === 0) return;
    const selectedFiles = [...files];
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

  const handleKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (showCommands) {
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        const direction = event.key === "ArrowDown" ? 1 : -1;
        setSelectedCommandIndex((current) =>
          (current + direction + commandCandidates.length) % commandCandidates.length,
        );
        return;
      }
      if (event.key === "Escape") {
        event.preventDefault();
        setCommandsDismissed(true);
        return;
      }
      if ((event.key === "Enter" || event.key === "Tab") && !event.nativeEvent.isComposing) {
        event.preventDefault();
        const command = commandCandidates[selectedCommandIndex];
        if (command !== undefined) selectCommand(command);
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

  useEffect(() => setSelectedCommandIndex(0), [commandQuery, commandCandidates.length]);

  useEffect(() => {
    if (!showCommands) return;
    commandOptionRefs.current[selectedCommandIndex]?.scrollIntoView?.({ block: "nearest" });
  }, [selectedCommandIndex, showCommands]);

  return (
    <div data-slot="composer" className="relative flex flex-col rounded-xl border border-border bg-card shadow-[0_1px_3px_rgba(0,0,0,0.06),0_8px_24px_rgba(0,0,0,0.04)] transition-[border-color,box-shadow] duration-200 hover:border-foreground/20 hover:shadow-[0_2px_4px_rgba(0,0,0,0.06),0_10px_28px_rgba(0,0,0,0.06)] focus-within:border-foreground/30 focus-within:shadow-[0_2px_4px_rgba(0,0,0,0.07),0_12px_32px_rgba(0,0,0,0.07)] focus-within:ring-2 focus-within:ring-ring/25 dark:shadow-[0_1px_3px_rgba(0,0,0,0.28),0_10px_28px_rgba(0,0,0,0.18)]">
      {showCommands && (
        <div
          id={commandListId}
          role="listbox"
          aria-label={t("chat.commands.available")}
          className="absolute inset-x-0 bottom-[calc(100%+8px)] z-40 overflow-hidden rounded-md border border-border bg-popover text-popover-foreground shadow-xl ring-1 ring-foreground/5"
        >
          <div className="flex h-9 items-center gap-2 border-b border-border/70 px-3 text-[11px] font-medium text-muted-foreground">
            <IconCommand className="size-3.5" aria-hidden="true" />
            {t("chat.commands.available")}
            <span className="ml-auto tabular-nums">{commandCandidates.length}</span>
          </div>
          <div className="max-h-64 overflow-y-auto overscroll-contain p-1.5 scroll-py-1.5">
            {commandCandidates.map((command, index) => (
              <button
                ref={(node) => { commandOptionRefs.current[index] = node; }}
                key={command.name}
                id={`${commandListId}-${index}`}
                type="button"
                role="option"
                aria-selected={index === selectedCommandIndex}
                onMouseDown={(event) => event.preventDefault()}
                onMouseMove={() => setSelectedCommandIndex(index)}
                onClick={() => selectCommand(command)}
                className="group flex min-h-12 w-full cursor-pointer items-center gap-3 rounded-md border border-transparent px-3 py-2 text-left outline-none transition-colors duration-150 hover:bg-accent/70 focus-visible:ring-2 focus-visible:ring-ring aria-selected:border-border aria-selected:bg-accent aria-selected:text-accent-foreground"
              >
                <span className="flex size-7 shrink-0 items-center justify-center rounded-md bg-muted font-mono text-xs font-semibold text-sky-700 transition-colors group-aria-selected:bg-background dark:text-sky-400">/</span>
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-xs font-semibold">{command.name}</span>
                  <span className="mt-0.5 block truncate text-[11px] text-muted-foreground">{command.description}</span>
                </span>
                {command.input && <span className="max-w-40 shrink truncate rounded bg-muted px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground">{command.input.hint}</span>}
              </button>
            ))}
          </div>
        </div>
      )}
      <div className="flex flex-col p-2">
        {attachments.length > 0 && (
          <div className="flex gap-2 overflow-x-auto px-2 pb-2 pt-1" aria-label={t("chat.attachments.selected")}>
            {attachments.map((attachment) => (
              <figure key={attachment.id} className="group/attachment relative size-16 shrink-0 overflow-hidden rounded-md border border-border bg-muted">
                <img src={`data:${attachment.content.mimeType};base64,${attachment.content.data}`} alt={attachment.name} className="size-full object-cover" />
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
            setCommandsDismissed(false);
          }}
          onKeyDown={handleKeyDown}
          aria-label={t("chat.messageLabel")}
          aria-autocomplete={availableCommands.length > 0 ? "list" : undefined}
          aria-haspopup={availableCommands.length > 0 ? "listbox" : undefined}
          aria-expanded={showCommands}
          aria-controls={showCommands ? commandListId : undefined}
          aria-activedescendant={showCommands ? `${commandListId}-${selectedCommandIndex}` : undefined}
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
            <Button type="button" variant="ghost" size="icon-sm" disabled={disabled || isResponding} aria-label={t("chat.attachments.add")} onClick={() => fileInputRef.current?.click()} className="rounded-full text-muted-foreground">
              <IconPlus className="size-4" />
            </Button>
            {attachments.length > 0 && <IconPhoto className="size-3.5 text-sky-600 dark:text-sky-400" aria-hidden="true" />}
            <PermissionSelector disabled={disabled} />
            <WorkflowToggle disabled={disabled} />
            {modes !== null && onModeChange !== undefined && <ModeSelector modes={modes} disabled={disabled || isResponding} onChange={onModeChange} />}
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
