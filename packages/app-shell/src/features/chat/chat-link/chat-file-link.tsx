import { useEffect, useState, type ReactNode } from "react";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
  toast,
} from "@ora/ui";
import { usePlatform } from "../../../platform";
import { useTranslation } from "react-i18next";
import { joinOsAbsolutePath } from "../../../lib/workspace-path";
import { ChatExternalLink } from "../chat-external-link";
import { useTaskChangesNavigation } from "../../diff/task-changes-navigation-context";
import { classifyChatCandidate, type ChatLinkClassification } from "./classify";
import { useChatLinkContext } from "./context";

const INLINE_CODE_CLASS =
  "rounded-sm border border-border/70 bg-muted/80 px-1.5 py-[0.15em] font-mono text-[0.85em]";

/** Codex-style file citation: blue path text and a dashed underline on hover. */
const CHAT_FILE_LINK_CLASS =
  "inline cursor-pointer border-0 bg-transparent p-0 font-mono text-[0.85em] font-normal text-sky-700 no-underline decoration-sky-700 decoration-dashed underline-offset-[3px] hover:underline focus-visible:rounded-sm focus-visible:underline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring dark:text-sky-400 dark:decoration-sky-400";

const CHAT_FILE_LINK_CODE_CLASS =
  "border-0 bg-transparent p-0 font-[inherit] text-[length:inherit] text-inherit leading-[inherit]";

export interface ChatFileLinkProps {
  source: "inline-code" | "href";
  raw: string;
  children: ReactNode;
  className?: string;
  /** When classification misses, inline code stays a chip unless this is `text`. */
  unmatched?: "code" | "text";
}

type FileLinkClassification = Extract<
  ChatLinkClassification,
  { kind: "diff" | "files" }
>;

/** Opens the classified in-app target for a chat file mention. */
function openClassified(
  classified: FileLinkClassification,
  navigation: NonNullable<ReturnType<typeof useTaskChangesNavigation>>,
) {
  if (classified.kind === "diff") {
    navigation.openDiff(classified.path, classified.line);
    return;
  }
  navigation.openWorkspaceFile(
    classified.path,
    classified.line,
    classified.column,
  );
}

/**
 * Focusable chat artifact control: left click routes by role, right click offers
 * OS handoff. Diff links also offer Preview in Files; Files links have no extra
 * in-app item because left click already opens Files.
 *
 * Platform is read only after a candidate classifies as a file link so tests
 * that render tool locations without a PlatformProvider keep working.
 */
export function ChatFileLink({
  source,
  raw,
  children,
  className,
  unmatched = "code",
}: ChatFileLinkProps) {
  const chatLink = useChatLinkContext();
  const navigation = useTaskChangesNavigation();
  const classified = classifyChatCandidate({
    source,
    raw,
    index: chatLink?.index ?? { edited: [], referenced: [] },
    hasNavigation: navigation !== null && chatLink !== null,
    cwd: chatLink?.cwd,
  });

  if (classified.kind === "none" || chatLink === null || navigation === null) {
    if (source === "inline-code" && unmatched === "code") {
      return <code className={className ?? INLINE_CODE_CLASS}>{children}</code>;
    }
    return <>{children}</>;
  }

  if (classified.kind === "web") {
    return (
      <ChatExternalLink
        className={
          className ??
          "font-medium text-primary underline decoration-primary/45 underline-offset-4 transition-colors hover:decoration-primary focus-visible:rounded-sm focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
        }
        href={classified.href}
      >
        {children}
      </ChatExternalLink>
    );
  }

  return (
    <LinkedChatFile
      source={source}
      raw={raw}
      className={className}
      initial={classified}
    >
      {children}
    </LinkedChatFile>
  );
}

/** Desktop cwd resolution and context menu live here so plain-code fallbacks stay platform-free. */
function LinkedChatFile({
  source,
  raw,
  className,
  initial,
  children,
}: {
  source: "inline-code" | "href";
  raw: string;
  className?: string;
  initial: FileLinkClassification;
  children: ReactNode;
}) {
  const { t } = useTranslation();
  const chatLink = useChatLinkContext()!;
  const navigation = useTaskChangesNavigation()!;
  const { locationActions } = usePlatform();
  const [desktopCwd, setDesktopCwd] = useState<string | null>(null);

  useEffect(() => {
    // Desktop is the only host: locationActions is always the cwd + OS-open
    // pair, not a supported/unsupported discriminant. Main-Workspace drafts rely
    // on MessageList's Workspace cwd instead of resolveTaskCwd.
    const taskId = chatLink.taskId;
    if (taskId === undefined) return;
    let cancelled = false;
    void locationActions
      .resolveTaskCwd(taskId)
      .then((path) => {
        if (cancelled) return;
        // Empty cwd is the same as "not resolved yet": keep null so tests and
        // first paint do not get a redundant setState after the stub resolves.
        const next = path.trim() === "" ? null : path;
        setDesktopCwd((current) => (current === next ? current : next));
      })
      .catch(() => {
        if (!cancelled) {
          setDesktopCwd((current) => (current === null ? current : null));
        }
      });
    return () => {
      cancelled = true;
    };
  }, [chatLink.taskId, locationActions]);

  const cwd = desktopCwd ?? chatLink.cwd ?? null;
  const refreshed = classifyChatCandidate({
    source,
    raw,
    index: chatLink.index,
    hasNavigation: true,
    cwd,
  });
  const classified: FileLinkClassification =
    refreshed.kind === "diff" || refreshed.kind === "files"
      ? refreshed
      : initial;

  // Explorer / VS Code / copy need a host path. Hide them until cwd is known
  // rather than handing a worktree-relative path to the OS.
  const osPath =
    cwd === null ? null : joinOsAbsolutePath(classified.displayPath, cwd);
  const ariaLabel = t("chat.fileLink.aria", { path: classified.path });
  const linkClassName = [CHAT_FILE_LINK_CLASS, className]
    .filter((part) => part !== undefined && part !== "")
    .join(" ");
  const showPreviewInFiles = classified.kind === "diff";

  const openOs = async (target: "explorer" | "vscode") => {
    if (osPath === null) return;
    try {
      await locationActions.open(target, osPath);
    } catch {
      toast.error(
        t("locationActions.openFailed", {
          app: t(
            target === "explorer"
              ? "locationActions.explorer"
              : "locationActions.vscode",
          ),
        }),
      );
    }
  };

  const copyPath = async () => {
    if (osPath === null) return;
    try {
      await navigator.clipboard.writeText(osPath);
      toast.success(t("locationActions.copied"));
    } catch {
      toast.error(t("locationActions.copyFailed"));
    }
  };

  return (
    <ContextMenu>
      <ContextMenuTrigger
        render={
          <button
            type="button"
            className={linkClassName}
            title={classified.displayPath}
            aria-label={ariaLabel}
            onClick={() => openClassified(classified, navigation)}
          />
        }
      >
        {source === "inline-code" ? (
          <code className={CHAT_FILE_LINK_CODE_CLASS}>{children}</code>
        ) : (
          children
        )}
      </ContextMenuTrigger>
      <ContextMenuContent>
        {osPath !== null && (
          <>
            <ContextMenuItem onClick={() => void openOs("explorer")}>
              {t("locationActions.explorer")}
            </ContextMenuItem>
            <ContextMenuItem onClick={() => void openOs("vscode")}>
              {t("locationActions.vscode")}
            </ContextMenuItem>
            <ContextMenuItem onClick={() => void copyPath()}>
              {t("locationActions.copyPath")}
            </ContextMenuItem>
          </>
        )}
        {osPath !== null && showPreviewInFiles && <ContextMenuSeparator />}
        {showPreviewInFiles && (
          <ContextMenuItem
            onClick={() =>
              navigation.openWorkspaceFile(
                classified.path,
                classified.line,
                classified.column,
              )
            }
          >
            {t("chat.fileLink.previewInFiles")}
          </ContextMenuItem>
        )}
      </ContextMenuContent>
    </ContextMenu>
  );
}
