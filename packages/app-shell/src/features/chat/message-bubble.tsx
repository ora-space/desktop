import { useState } from "react";
import { IconCheck, IconCopy, IconThumbDown, IconThumbUp } from "@tabler/icons-react";
import { Button } from "@ora/ui";
import { useTranslation } from "react-i18next";
import { OraMark } from "../../components/ora-mark";
import { formatClock } from "../../lib/format";
import { AnchorHighlight } from "./anchor-highlight";
import { MarkdownMessage } from "./markdown-message";
import type { ChatMessage } from "@ora/chat";

interface MessageBubbleProps {
  message: ChatMessage;
  userName: string;
  embeddedAssistant?: boolean;
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
export function MessageBubble({ message, userName, embeddedAssistant = false }: MessageBubbleProps) {
  const { t } = useTranslation();
  const { copied, copy } = useCopyMessage(message.content);
  const isUser = message.role === "user";

  return (
    <div className={`group/message flex gap-3 ${embeddedAssistant ? "py-1" : "py-5"} ${isUser ? "justify-end" : "justify-start"}`}>
      {!isUser && !embeddedAssistant && <OraMark size="sm" />}

      <div className={`flex min-w-0 flex-col gap-1.5 ${isUser ? "max-w-[85%] items-end" : "flex-1"}`}>
        {isUser ? (
          <div className="relative w-fit max-w-full rounded-2xl rounded-br-md bg-secondary px-4 py-2.5">
            <AnchorHighlight />
            <p className="relative whitespace-pre-wrap break-words text-[14px] leading-6 text-foreground">{message.content}</p>
          </div>
        ) : (
          <MarkdownMessage content={message.content} />
        )}

        <div className={`flex min-h-6 items-center gap-2 ${isUser ? "pr-1" : ""}`}>
          <span className="text-xs text-muted-foreground">{formatClock(message.createdAt)}</span>
          {!isUser && (
            <div className="flex items-center gap-0.5 opacity-0 transition-opacity duration-150 group-hover/message:opacity-100 group-focus-within/message:opacity-100">
              <Button variant="ghost" size="icon-xs" aria-label={t("chat.copy")} onClick={copy}>
                {copied ? (
                  <IconCheck className="size-3.5 text-emerald-600" />
                ) : (
                  <IconCopy className="size-3.5 text-muted-foreground" />
                )}
              </Button>
              <Button variant="ghost" size="icon-xs" aria-label={t("chat.goodResponse")}>
                <IconThumbUp className="size-3.5 text-muted-foreground" />
              </Button>
              <Button variant="ghost" size="icon-xs" aria-label={t("chat.badResponse")}>
                <IconThumbDown className="size-3.5 text-muted-foreground" />
              </Button>
            </div>
          )}
        </div>
      </div>

      <span className="sr-only">{isUser ? `${userName}: ${t("chat.youSaid")}` : t("chat.assistantReplied")}</span>
    </div>
  );
}
