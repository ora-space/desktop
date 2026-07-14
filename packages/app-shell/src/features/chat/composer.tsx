import { useEffect, useRef, useState } from "react";
import type { KeyboardEvent } from "react";
import { ArrowUp } from "@untitledui/icons";
import { Button, TextArea } from "@ora/ui";

interface ComposerProps {
  onSend: (text: string) => void;
  isResponding: boolean;
  placeholder?: string;
  autoFocus?: boolean;
}

/**
 * The chat composer: a rounded input shell wrapping the @ora/ui TextArea with
 * an inline send button. Enter sends, Shift+Enter inserts a newline, and the
 * textarea auto-grows up to a max height.
 */
export function Composer({ onSend, isResponding, placeholder = "Message Ora…", autoFocus = false }: ComposerProps) {
  const [value, setValue] = useState("");
  const textAreaRef = useRef<HTMLTextAreaElement>(null);

  const canSend = value.trim().length > 0 && !isResponding;

  const submit = () => {
    const text = value.trim();
    if (!text || isResponding) return;
    onSend(text);
    setValue("");
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
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
    <div className="flex flex-col rounded-2xl bg-primary p-2 shadow-xs ring-1 ring-secondary transition focus-within:ring-2 focus-within:ring-brand">
      <TextArea
        textAreaRef={textAreaRef}
        autoFocus={autoFocus}
        placeholder={placeholder}
        value={value}
        onChange={setValue}
        onKeyDown={handleKeyDown}
        textAreaClassName="resize-none rounded-none bg-transparent px-2 py-1.5 shadow-none ring-0 min-h-[28px] max-h-[200px]"
      />
      <div className="flex items-center justify-between pt-1">
        <p className="px-2 text-xs text-quaternary">
          Enter to send · <span className="text-tertiary">Shift+Enter for newline</span>
        </p>
        <Button
          color="primary"
          size="sm"
          aria-label="Send message"
          isDisabled={!canSend}
          onClick={submit}
          noTextPadding
          className="size-8 rounded-full p-0"
        >
          <ArrowUp className="size-[18px] text-white" />
        </Button>
      </div>
    </div>
  );
}
