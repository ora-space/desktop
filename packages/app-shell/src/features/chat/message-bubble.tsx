import { useState } from "react";
import { Check, Copy07, ThumbsDown, ThumbsUp } from "@untitledui/icons";
import { Button } from "@ora/ui";
import { ColoredAvatar } from "../../components/colored-avatar";
import { OraMark } from "../../components/ora-mark";
import { formatClock } from "../../lib/format";
import type { ChatMessage } from "../../lib/types";

interface MessageBubbleProps {
  message: ChatMessage;
  userName: string;
}

/** Copies message content to the clipboard and briefly confirms with a check. */
function useCopyMessage(content: string) {
  const [copied, setCopied] = useState(false);

  const copy = () => {
    navigator.clipboard.writeText(content).then(() => {
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    });
  };

  return { copied, copy };
}

/** A single chat message: avatar + content, with hover actions on replies. */
export function MessageBubble({ message, userName }: MessageBubbleProps) {
  const { copied, copy } = useCopyMessage(message.content);
  const isUser = message.role === "user";

  return (
    <div className="group/message flex gap-3 py-4">
      {isUser ? <ColoredAvatar name={userName} size="sm" /> : <OraMark size="sm" />}

      <div className="flex min-w-0 flex-1 flex-col gap-1.5">
        {isUser ? (
          <div className="w-fit max-w-full rounded-2xl bg-secondary px-3.5 py-2.5">
            <p className="whitespace-pre-wrap break-words text-md text-primary">{message.content}</p>
          </div>
        ) : (
          <p className="whitespace-pre-wrap break-words text-md text-primary">{message.content}</p>
        )}

        <div className="flex items-center gap-2">
          <span className="text-xs text-quaternary">{formatClock(message.createdAt)}</span>
          {!isUser && (
            <div className="flex items-center gap-0.5 opacity-0 transition duration-100 group-hover/message:opacity-100">
              <Button color="tertiary" size="sm" aria-label="Copy" noTextPadding className="size-6 p-0" onClick={copy}>
                {copied ? (
                  <Check className="size-3.5 text-fg-success-secondary" />
                ) : (
                  <Copy07 className="size-3.5 text-fg-quaternary transition-inherit-all group-hover/message:text-fg-tertiary_hover" />
                )}
              </Button>
              <Button color="tertiary" size="sm" aria-label="Good response" noTextPadding className="size-6 p-0">
                <ThumbsUp className="size-3.5 text-fg-quaternary transition-inherit-all group-hover/message:text-fg-tertiary_hover" />
              </Button>
              <Button color="tertiary" size="sm" aria-label="Bad response" noTextPadding className="size-6 p-0">
                <ThumbsDown className="size-3.5 text-fg-quaternary transition-inherit-all group-hover/message:text-fg-tertiary_hover" />
              </Button>
            </div>
          )}
        </div>
      </div>

      <span className="sr-only">{isUser ? "You said" : "Assistant replied"}</span>
    </div>
  );
}
