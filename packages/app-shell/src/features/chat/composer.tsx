import { useEffect, useRef, useState } from "react";
import type { KeyboardEvent } from "react";
import { IconArrowUp, IconPlayerStop, IconPlus } from "@tabler/icons-react";
import { Button, Textarea } from "@ora/ui";
import { useTranslation } from "react-i18next";
import { ModelSelector } from "./model-selector";
import { PermissionSelector } from "./permission-selector";

interface ComposerProps {
  onSend: (text: string) => void;
  onStop?: () => void;
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
  onStop,
  isResponding,
  disabled = false,
  placeholder,
  autoFocus = false,
}: ComposerProps) {
  const { t } = useTranslation();
  const [value, setValue] = useState("");
  const textAreaRef = useRef<HTMLTextAreaElement>(null);

  const canSend = value.trim().length > 0 && !isResponding && !disabled;

  const submit = () => {
    const text = value.trim();
    if (!text || isResponding || disabled) return;
    onSend(text);
    setValue("");
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
    <div data-slot="composer" className="flex flex-col rounded-xl border border-border/70 bg-card shadow-[0_1px_2px_rgba(0,0,0,0.04)] transition-[border-color,box-shadow] duration-200 hover:border-border hover:shadow-[0_3px_10px_rgba(0,0,0,0.07)] focus-within:border-foreground/25 focus-within:shadow-[0_3px_12px_rgba(0,0,0,0.08)] focus-within:ring-1 focus-within:ring-ring/35 dark:shadow-[0_1px_2px_rgba(0,0,0,0.2)] dark:hover:shadow-[0_3px_12px_rgba(0,0,0,0.24)]">
      <div className="flex flex-col p-2">
        <Textarea
          ref={textAreaRef}
          autoFocus={autoFocus}
          placeholder={placeholder ?? t("chat.placeholder")}
          value={value}
          disabled={disabled}
          onChange={(event) => setValue(event.target.value)}
          onKeyDown={handleKeyDown}
          aria-label={t("chat.messageLabel")}
          // The shell already carries the surface, so the Textarea's own disabled
          // fill would read as a grey block floating inside the card.
          className="min-h-14 max-h-[200px] resize-none rounded-none border-0 bg-transparent px-2 py-1 text-[15px] leading-6 shadow-none focus-visible:ring-0 disabled:bg-transparent"
        />
        <div className="flex min-h-8 items-center justify-between gap-2 pt-0.5">
          <div className="flex min-w-0 items-center gap-1">
            {/* Placeholder affordance: the add button is intentionally inert for now. */}
            <Button type="button" variant="ghost" size="icon-sm" disabled={disabled} aria-label={t("chat.add")} className="rounded-full text-muted-foreground">
              <IconPlus className="size-4" />
            </Button>
            <PermissionSelector disabled={disabled} />
          </div>
          <div className="flex shrink-0 items-center gap-2">
            <ModelSelector disabled={disabled} />
            <Button
              size="icon"
              aria-label={isResponding ? t("common.stop") : t("chat.send")}
              disabled={isResponding ? onStop === undefined : !canSend}
              onClick={isResponding ? onStop : submit}
              className="size-8 rounded-full disabled:bg-muted disabled:text-muted-foreground"
            >
              {isResponding ? <IconPlayerStop className="size-[18px]" /> : <IconArrowUp className="size-[18px]" />}
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
}
