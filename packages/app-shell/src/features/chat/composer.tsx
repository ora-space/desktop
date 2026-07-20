import { useEffect, useRef, useState } from "react";
import type { KeyboardEvent } from "react";
import { IconArrowUp, IconCheck, IconChevronDown, IconPaperclip, IconSparkles, IconX } from "@tabler/icons-react";
import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Textarea,
} from "@ora/ui";
import { useTranslation } from "react-i18next";

interface ComposerProps {
  onSend: (text: string) => void;
  isResponding: boolean;
  disabled?: boolean;
  placeholder?: string;
  autoFocus?: boolean;
}

/**
 * The chat composer: a rounded input shell wrapping the @ora/ui Textarea with
 * an inline send button. Enter sends, Shift+Enter inserts a newline, and the
 * textarea auto-grows up to a max height.
 */
export function Composer({
  onSend,
  isResponding,
  disabled = false,
  placeholder,
  autoFocus = false,
}: ComposerProps) {
  const { t } = useTranslation();
  const [value, setValue] = useState("");
  const [attachments, setAttachments] = useState<string[]>([]);
  const [mode, setMode] = useState<"agent" | "chat">("agent");
  const [environment, setEnvironment] = useState<"local" | "cloud">("local");
  const textAreaRef = useRef<HTMLTextAreaElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);

  const canSend = (value.trim().length > 0 || attachments.length > 0) && !isResponding && !disabled;

  const submit = () => {
    const text = value.trim();
    if ((!text && attachments.length === 0) || isResponding || disabled) return;
    const attachmentReferences = attachments.map((fileName) => `@${fileName}`).join(" ");
    onSend([text, attachmentReferences].filter(Boolean).join("\n"));
    setValue("");
    setAttachments([]);
  };

  /** Adds unique file references without reading local file contents into the prototype. */
  const addAttachments = (files: FileList | null) => {
    if (!files) return;
    setAttachments((current) => [...new Set([...current, ...Array.from(files, (file) => file.name)])]);
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
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

  return (
    <div className="flex flex-col rounded-2xl border border-border/90 bg-card p-2 shadow-[0_8px_30px_rgba(0,0,0,0.06)] transition-shadow duration-200 focus-within:border-foreground/25 focus-within:shadow-[0_10px_36px_rgba(0,0,0,0.09)] focus-within:ring-2 focus-within:ring-ring/25 dark:shadow-[0_8px_30px_rgba(0,0,0,0.22)]">
      <Textarea
        ref={textAreaRef}
        autoFocus={autoFocus}
        placeholder={placeholder ?? t("chat.placeholder")}
        value={value}
        disabled={disabled}
        onChange={(event) => setValue(event.target.value)}
        onKeyDown={handleKeyDown}
        aria-label={t("chat.messageLabel")}
        className="min-h-[68px] max-h-[200px] resize-none rounded-none border-0 bg-transparent px-2 py-1.5 text-[15px] leading-6 shadow-none focus-visible:ring-0"
      />
      {attachments.length > 0 && (
        <div className="flex flex-wrap gap-1.5 px-2 pb-1" aria-label={t("chat.attachments")}>
          {attachments.map((fileName) => (
            <span key={fileName} className="inline-flex h-7 max-w-52 items-center gap-1 rounded-md bg-muted px-2 text-xs text-muted-foreground">
              <span className="truncate">{fileName}</span>
              <button
                type="button"
                aria-label={t("chat.removeAttachment", { fileName })}
                onClick={() => setAttachments((current) => current.filter((candidate) => candidate !== fileName))}
                className="rounded-sm outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
              >
                <IconX className="size-3" />
              </button>
            </span>
          ))}
        </div>
      )}
      <div className="flex min-h-8 items-center justify-between gap-2 pt-1">
        <div className="flex min-w-0 items-center gap-0.5">
          <input
            ref={fileInputRef}
            type="file"
            multiple
            className="sr-only"
            tabIndex={-1}
            onChange={(event) => {
              addAttachments(event.target.files);
              event.target.value = "";
            }}
          />
          <Button type="button" variant="ghost" size="icon-sm" disabled={disabled} onClick={() => fileInputRef.current?.click()} aria-label={t("chat.attach")} className="rounded-full text-muted-foreground">
            <IconPaperclip className="size-4" />
          </Button>
          <DropdownMenu>
            <DropdownMenuTrigger render={<Button type="button" variant="ghost" size="sm" className="gap-1 px-2 text-xs font-normal text-muted-foreground" />}>
              <IconSparkles className="size-3.5" />
              {mode === "agent" ? t("chat.agentMode") : t("chat.chatMode")}
              <IconChevronDown className="size-3" />
            </DropdownMenuTrigger>
            <DropdownMenuContent align="start" side="top" className="w-44">
              <DropdownMenuItem onClick={() => setMode("agent")}><IconSparkles />{t("chat.agentMode")}{mode === "agent" && <IconCheck className="ml-auto" />}</DropdownMenuItem>
              <DropdownMenuItem onClick={() => setMode("chat")}><IconSparkles />{t("chat.chatMode")}{mode === "chat" && <IconCheck className="ml-auto" />}</DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
          <DropdownMenu>
            <DropdownMenuTrigger render={<Button type="button" variant="ghost" size="sm" className="hidden gap-1 px-2 text-xs font-normal text-muted-foreground sm:inline-flex" />}>
              {environment === "local" ? t("chat.local") : t("chat.cloud")}
              <IconChevronDown className="size-3" />
            </DropdownMenuTrigger>
            <DropdownMenuContent align="start" side="top" className="w-40">
              <DropdownMenuItem onClick={() => setEnvironment("local")}>{t("chat.local")}{environment === "local" && <IconCheck className="ml-auto" />}</DropdownMenuItem>
              <DropdownMenuItem onClick={() => setEnvironment("cloud")}>{t("chat.cloud")}{environment === "cloud" && <IconCheck className="ml-auto" />}</DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <p className="hidden text-[11px] text-muted-foreground lg:block">{t("chat.sendHint")}</p>
          <Button
            size="icon"
            aria-label={t("chat.send")}
            disabled={!canSend}
            onClick={submit}
            className="size-8 rounded-full disabled:bg-muted disabled:text-muted-foreground"
          >
            <IconArrowUp className="size-[18px]" />
          </Button>
        </div>
      </div>
    </div>
  );
}
