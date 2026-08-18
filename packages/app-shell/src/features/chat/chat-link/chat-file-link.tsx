import { useEffect, useState, type ReactNode } from "react";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
  toast,
} from "@ora/ui";
import { usePlatform } from "@ora/platform";
import { useTranslation } from "react-i18next";
import { joinOsAbsolutePath } from "../../../lib/workspace-path";
import { useTaskChangesNavigation } from "../../diff/task-changes-navigation-context";
import {
  classifyChatCandidate,
  matchIndexPath,
  type ChatLinkClassification,
} from "./classify";
import { useChatLinkContext } from "./context";

const INLINE_CODE_CLASS =
  "rounded-sm border border-border/70 bg-muted/80 px-1.5 py-[0.15em] font-mono text-[0.85em]";

export interface ChatFileLinkProps {
  source: "inline-code" | "href";
  raw: string;
  children: ReactNode;
  className?: string;
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
 * OS handoff and the in-app surface the left click did not use.
 *
 * Platform is read only after a candidate classifies as a file link so tests
 * that render tool locations without a PlatformProvider keep working.
 */
export function ChatFileLink({
  source,
  raw,
  children,
  className,
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
    return source === "inline-code" ? (
      <code className={className ?? INLINE_CODE_CLASS}>{children}</code>
    ) : (
      <>{children}</>
    );
  }

  if (classified.kind === "web") {
    return (
      <a
        className={
          className ??
          "font-medium text-primary underline decoration-primary/45 underline-offset-4 transition-colors hover:decoration-primary focus-visible:rounded-sm focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
        }
        href={classified.href}
        rel="noopener noreferrer"
        target="_blank"
      >
        {children}
      </a>
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
    if (locationActions.kind !== "supported") return;
    let cancelled = false;
    void locationActions
      .resolveTaskCwd(chatLink.taskId)
      .then((path) => {
        if (!cancelled) setDesktopCwd(path);
      })
      .catch(() => {
        if (!cancelled) setDesktopCwd(null);
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

  const desktop = locationActions.kind === "supported";
  const editedHit = matchIndexPath(classified.path, chatLink.index.edited);
  const osPath =
    cwd === null
      ? classified.path
      : joinOsAbsolutePath(classified.displayPath, cwd);
  const ariaLabel = t("chat.fileLink.aria", { path: classified.path });
  const linkClassName =
    source === "inline-code"
      ? `${INLINE_CODE_CLASS} cursor-pointer text-primary underline decoration-primary/45 underline-offset-4`
      : "cursor-pointer font-medium text-primary underline decoration-primary/45 underline-offset-4 transition-colors hover:decoration-primary focus-visible:rounded-sm focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring";
  const showInAppAlternate =
    classified.kind === "diff" ||
    (classified.kind === "files" && editedHit !== null);

  const openOs = async (target: "explorer" | "vscode") => {
    if (locationActions.kind !== "supported") return;
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
            className={className ?? linkClassName}
            title={classified.displayPath}
            aria-label={ariaLabel}
            onClick={() => openClassified(classified, navigation)}
          />
        }
      >
        {source === "inline-code" ? <code>{children}</code> : children}
      </ContextMenuTrigger>
      <ContextMenuContent>
        {desktop && (
          <>
            <ContextMenuItem onClick={() => void openOs("explorer")}>
              {t("locationActions.explorer")}
            </ContextMenuItem>
            <ContextMenuItem onClick={() => void openOs("vscode")}>
              {t("locationActions.vscode")}
            </ContextMenuItem>
          </>
        )}
        <ContextMenuItem onClick={() => void copyPath()}>
          {t("locationActions.copyPath")}
        </ContextMenuItem>
        {desktop && showInAppAlternate && <ContextMenuSeparator />}
        {classified.kind === "diff" && (
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
        {classified.kind === "files" && editedHit !== null && (
          <ContextMenuItem
            onClick={() =>
              navigation.openDiff(classified.path, classified.line)
            }
          >
            {t("chat.fileLink.viewInChanges")}
          </ContextMenuItem>
        )}
      </ContextMenuContent>
    </ContextMenu>
  );
}
