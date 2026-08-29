import { useState } from "react";
import {
  IconCheck,
  IconCopy,
  IconThumbDown,
  IconThumbUp,
} from "@tabler/icons-react";
import { Button } from "@ora/ui";
import { useTranslation } from "react-i18next";
import type { ChatMessage } from "@ora/chat";
import type * as acp from "@agentclientprotocol/sdk";
import { formatClock } from "../../lib/format";
import { AnchorHighlight } from "./anchor-highlight";
import { ContentBlock } from "./content-block";
import { MarkdownDocument, MarkdownMessage } from "./markdown-message";

interface MessageBubbleProps {
  message: ChatMessage;
  userName: string;
  embeddedAssistant?: boolean;
  streaming?: boolean;
  /** Tighter rhythm for read-only embedded conversations such as workflow cards. */
  compact?: boolean;
  /** Lets an embedding surface own the highlight geometry for the whole message row. */
  showAnchorHighlight?: boolean;
}

/**
 * Read-only user body today; `mode: "edit"` is reserved for mounting
 * ComposerEditor on the same `documentPlainText` string later.
 */
type UserMessageBodyMode = "view" | "edit";

interface UserMessageBodyProps {
  content: string;
  structuredContent?: Array<Exclude<acp.ContentBlock, { type: "text" }>>;
  messageId: string;
  showAnchorHighlight: boolean;
  /** Edit mounts ComposerEditor; only view is wired this release. */
  mode?: UserMessageBodyMode;
}

/**
 * User prompt surface: MarkdownDocument for history, TipTap Composer when
 * editing is enabled. Persistence stays `documentPlainText` either way.
 */
function UserMessageBody({
  content,
  structuredContent,
  messageId,
  showAnchorHighlight,
  mode = "view",
}: UserMessageBodyProps) {
  return (
    <>
      {structuredContent?.map((block, index) => (
        <ContentBlock key={`${messageId}-content-${index}`} content={block} />
      ))}
      {content.length > 0 && mode === "view" && (
        <div className="relative w-fit max-w-full overflow-visible rounded-2xl rounded-br-md bg-secondary px-4 py-2.5">
          {showAnchorHighlight && <AnchorHighlight />}
          <div className="relative">
            <MarkdownDocument content={content} density="compact" />
          </div>
        </div>
      )}
      {/* mode === "edit" -> ComposerEditor(initialText=content) when edit ships */}
    </>
  );
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

/** A single chat message: avatar + content, with hover copy on both roles. */
export function MessageBubble({
  message,
  userName,
  embeddedAssistant = false,
  streaming = false,
  compact = false,
  showAnchorHighlight = true,
}: MessageBubbleProps) {
  const { t } = useTranslation();
  const { copied, copy } = useCopyMessage(message.content);
  const isUser = message.role === "user";
  const canCopy = message.content.length > 0;

  return (
    <div
      className={`group/message flex gap-3 ${compact || embeddedAssistant ? "py-1.5" : "py-5"} ${isUser ? "justify-end" : "justify-start"}`}
    >
      <div
        className={`flex min-w-0 flex-col gap-1.5 ${isUser ? "max-w-[85%] items-end" : "flex-1"}`}
      >
        {isUser ? (
          <UserMessageBody
            content={message.content}
            structuredContent={message.structuredContent}
            messageId={message.id}
            showAnchorHighlight={showAnchorHighlight}
          />
        ) : (
          <div className="relative">
            {showAnchorHighlight && <AnchorHighlight />}
            <MarkdownMessage content={message.content} streaming={streaming} />
          </div>
        )}

        {!compact && (
          <div
            className={`flex min-h-6 items-center gap-2 ${isUser ? "flex-row-reverse pr-1" : ""}`}
          >
            <span className="text-xs text-muted-foreground">
              {formatClock(message.createdAt)}
            </span>
            <div className="flex items-center gap-0.5 opacity-0 transition-opacity duration-150 group-hover/message:opacity-100 group-focus-within/message:opacity-100">
              {canCopy && (
                <Button
                  variant="ghost"
                  size="icon-xs"
                  aria-label={t("chat.copy")}
                  onClick={copy}
                >
                  {copied ? (
                    <IconCheck className="size-3.5 text-emerald-600" />
                  ) : (
                    <IconCopy className="size-3.5 text-muted-foreground" />
                  )}
                </Button>
              )}
              {!isUser && (
                <>
                  <Button
                    variant="ghost"
                    size="icon-xs"
                    aria-label={t("chat.goodResponse")}
                  >
                    <IconThumbUp className="size-3.5 text-muted-foreground" />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon-xs"
                    aria-label={t("chat.badResponse")}
                  >
                    <IconThumbDown className="size-3.5 text-muted-foreground" />
                  </Button>
                </>
              )}
            </div>
          </div>
        )}
      </div>

      <span className="sr-only">
        {isUser
          ? `${userName}: ${t("chat.youSaid")}`
          : t("chat.assistantReplied")}
      </span>
    </div>
  );
}
