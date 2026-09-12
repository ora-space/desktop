import {
  createContext,
  useContext,
  useEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
} from "react";
import { useQuery } from "@tanstack/react-query";
import type { WorkspaceEntry } from "@ora/contracts";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
  Input,
  toast,
} from "@ora/ui";
import {
  IconChevronDown,
  IconChevronRight,
  IconClipboard,
  IconCopy,
  IconCut,
  IconFilePlus,
  IconFolder,
  IconFolderPlus,
  IconFolderShare,
  IconPencil,
  IconTrash,
} from "@tabler/icons-react";
import { useTranslation } from "react-i18next";
import { localizeContractError } from "../../i18n/contract-error";
import { useContractErrorToast } from "../../i18n/use-contract-error-toast";
import { joinOsAbsolutePath } from "../../lib/workspace-path";
import { usePlatform } from "../../platform";
import {
  directoryQueryKey,
  filesScopeApi,
  joinWorkspaceChild,
  parentPath,
  type FilesScope,
} from "../../state/data/files";
import { WorkspaceFileIcon } from "./workspace-file-visuals";

export interface FileCreateDraft {
  parentPath: string;
  kind: "file" | "directory";
}

export interface FileExplorerClipboard {
  mode: "copy" | "cut";
  path: string;
}

export interface FileExplorerSessionValue {
  cwd: string | undefined;
  clipboard: FileExplorerClipboard | null;
  createDraft: FileCreateDraft | null;
  renamePath: string | null;
  onBeginCreate: (parentPath: string, kind: "file" | "directory") => void;
  onCommitCreate: (path: string, kind: "file" | "directory") => Promise<void>;
  onCancelCreate: () => void;
  onBeginRename: (path: string) => void;
  onCommitRename: (from: string, name: string) => Promise<void>;
  onCancelRename: () => void;
  onCopy: (path: string) => void;
  onCut: (path: string) => void;
  onPaste: (
    targetPath: string,
    targetKind: "file" | "directory" | "root",
  ) => void;
  onDelete: (path: string, name: string, kind: "file" | "directory") => void;
}

const FileExplorerSessionContext =
  createContext<FileExplorerSessionValue | null>(null);

/** Supplies explorer clipboard, create, and rename actions to every tree row. */
export function FileExplorerSession({
  value,
  children,
}: {
  value: FileExplorerSessionValue;
  children: ReactNode;
}) {
  return (
    <FileExplorerSessionContext.Provider value={value}>
      {children}
    </FileExplorerSessionContext.Provider>
  );
}

function useFileExplorerSession() {
  const value = useContext(FileExplorerSessionContext);
  if (value === null) {
    throw new Error("FileExplorerSession is required");
  }
  return value;
}

interface DirectoryTreeProps {
  scope: FilesScope;
  scopeApi: ReturnType<typeof filesScopeApi>;
  path: string;
  depth: number;
  expanded: ReadonlySet<string>;
  selectedPath: string | null;
  onToggleDirectory: (path: string) => void;
  onSelectFile: (path: string) => void;
}

/** Loads one expanded directory lazily and renders its descendants recursively. */
export function DirectoryTree({
  scope,
  scopeApi,
  path,
  depth,
  expanded,
  selectedPath,
  onToggleDirectory,
  onSelectFile,
}: DirectoryTreeProps) {
  const { t } = useTranslation();
  const session = useFileExplorerSession();
  const directoryQuery = useQuery({
    queryKey: directoryQueryKey(scope, path),
    queryFn: ({ signal }) => scopeApi.listDirectory(path, signal),
  });

  if (directoryQuery.isLoading) {
    return (
      <p className="px-3 py-2 text-xs text-muted-foreground">
        {t("files.loading")}
      </p>
    );
  }
  if (directoryQuery.error) {
    return (
      <p className="px-3 py-2 text-xs text-destructive">
        {localizeContractError(directoryQuery.error, t)}
      </p>
    );
  }

  const showDraft =
    session.createDraft !== null && session.createDraft.parentPath === path;

  return (
    <>
      {showDraft && session.createDraft !== null && (
        <NameDraftRow
          depth={depth}
          kind={session.createDraft.kind}
          initialName=""
          ariaLabel={
            session.createDraft.kind === "directory"
              ? t("files.newFolder")
              : t("files.newFile")
          }
          onCommit={(name) =>
            session.onCommitCreate(
              joinWorkspaceChild(path, name),
              session.createDraft!.kind,
            )
          }
          onCancel={session.onCancelCreate}
        />
      )}
      {directoryQuery.data?.entries.map((entry) => (
        <WorkspaceTreeEntry
          key={entry.path}
          entry={entry}
          scope={scope}
          scopeApi={scopeApi}
          depth={depth}
          expanded={expanded}
          selectedPath={selectedPath}
          onToggleDirectory={onToggleDirectory}
          onSelectFile={onSelectFile}
        />
      ))}
    </>
  );
}

/** Renders one tree row and mounts its lazy child query only while expanded. */
function WorkspaceTreeEntry({
  entry,
  scope,
  scopeApi,
  depth,
  expanded,
  selectedPath,
  onToggleDirectory,
  onSelectFile,
}: Omit<DirectoryTreeProps, "path"> & { entry: WorkspaceEntry }) {
  const { t } = useTranslation();
  const session = useFileExplorerSession();
  const isDirectory = entry.kind === "directory";
  const isExpanded = isDirectory && expanded.has(entry.path);
  const createParent = isDirectory ? entry.path : parentPath(entry.path);
  const renaming = session.renamePath === entry.path;

  const onRowKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.nativeEvent.isComposing) return;
    const withMeta = event.ctrlKey || event.metaKey;
    if (event.key === "F2") {
      event.preventDefault();
      session.onBeginRename(entry.path);
      return;
    }
    if (withMeta && event.key.toLowerCase() === "c") {
      event.preventDefault();
      session.onCopy(entry.path);
      return;
    }
    if (withMeta && event.key.toLowerCase() === "x") {
      event.preventDefault();
      session.onCut(entry.path);
      return;
    }
    if (withMeta && event.key.toLowerCase() === "v") {
      event.preventDefault();
      session.onPaste(entry.path, entry.kind);
      return;
    }
    if (event.key === "Delete" || event.key === "Backspace") {
      event.preventDefault();
      session.onDelete(entry.path, entry.name, entry.kind);
      return;
    }
    // Keep native button activation (Enter/Space) without swallowing Tab.
    if (event.key !== "Enter" && event.key !== " ") return;
    event.preventDefault();
    if (isDirectory) onToggleDirectory(entry.path);
    else onSelectFile(entry.path);
  };

  return (
    <>
      <FileTreeContextMenu
        relativePath={entry.path}
        targetKind={entry.kind}
        onNewFile={() => session.onBeginCreate(createParent, "file")}
        onNewFolder={() => session.onBeginCreate(createParent, "directory")}
        onCopy={() => session.onCopy(entry.path)}
        onCut={() => session.onCut(entry.path)}
        onPaste={() => session.onPaste(entry.path, entry.kind)}
        onRename={() => session.onBeginRename(entry.path)}
        onDelete={() => session.onDelete(entry.path, entry.name, entry.kind)}
      >
        {renaming ? (
          <NameDraftRow
            depth={depth}
            kind={entry.kind}
            initialName={entry.name}
            ariaLabel={t("files.rename")}
            onCommit={(name) => session.onCommitRename(entry.path, name)}
            onCancel={session.onCancelRename}
          />
        ) : (
          <div
            role="button"
            tabIndex={0}
            aria-expanded={isDirectory ? isExpanded : undefined}
            aria-current={selectedPath === entry.path ? "page" : undefined}
            className={`flex h-7 w-full items-center gap-1 border-l-2 pr-2 text-left text-xs outline-none hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring ${
              selectedPath === entry.path
                ? "border-primary bg-accent/80 text-accent-foreground"
                : "border-transparent"
            }`}
            style={{ paddingLeft: `${8 + depth * 14}px` }}
            onClick={() =>
              isDirectory
                ? onToggleDirectory(entry.path)
                : onSelectFile(entry.path)
            }
            onKeyDown={onRowKeyDown}
          >
            {isDirectory ? (
              isExpanded ? (
                <IconChevronDown className="size-3.5" />
              ) : (
                <IconChevronRight className="size-3.5" />
              )
            ) : (
              <span className="w-3.5" />
            )}
            {isDirectory ? (
              <IconFolder className="size-4 shrink-0 text-amber-600" />
            ) : (
              <WorkspaceFileIcon path={entry.path} />
            )}
            <span className="truncate">{entry.name}</span>
          </div>
        )}
      </FileTreeContextMenu>
      {isExpanded && (
        <DirectoryTree
          scope={scope}
          scopeApi={scopeApi}
          path={entry.path}
          depth={depth + 1}
          expanded={expanded}
          selectedPath={selectedPath}
          onToggleDirectory={onToggleDirectory}
          onSelectFile={onSelectFile}
        />
      )}
    </>
  );
}

/** Empty-area menu for the explorer root, matching Cursor's blank-tree right click. */
export function FileTreeRootMenu({ children }: { children: ReactNode }) {
  const session = useFileExplorerSession();
  return (
    <FileTreeContextMenu
      relativePath=""
      targetKind="root"
      onNewFile={() => session.onBeginCreate("", "file")}
      onNewFolder={() => session.onBeginCreate("", "directory")}
      onPaste={() => session.onPaste("", "root")}
    >
      {children}
    </FileTreeContextMenu>
  );
}

/**
 * One explorer-row context menu: create, cut/copy/paste/rename/delete, copy paths, and reveal.
 *
 * The trigger is a host `div` with `preventDefault` on `contextmenu` so WebKit/WebView2
 * still delivers the event, matching the sidebar tree rows.
 */
function FileTreeContextMenu({
  relativePath,
  targetKind,
  children,
  onNewFile,
  onNewFolder,
  onCopy,
  onCut,
  onPaste,
  onRename,
  onDelete,
}: {
  relativePath: string;
  targetKind: "file" | "directory" | "root";
  children: ReactNode;
  onNewFile: () => void;
  onNewFolder: () => void;
  onCopy?: () => void;
  onCut?: () => void;
  onPaste: () => void;
  onRename?: () => void;
  onDelete?: () => void;
}) {
  const { t } = useTranslation();
  const { locationActions } = usePlatform();
  const session = useFileExplorerSession();
  const absolutePath =
    session.cwd === undefined
      ? null
      : joinOsAbsolutePath(relativePath, session.cwd);
  const relativeLabel = relativePath === "" ? "." : relativePath;
  const canEditEntry = targetKind !== "root";
  const canPaste = session.clipboard !== null;

  const copyText = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      toast.success(t("locationActions.copied"));
    } catch {
      toast.error(t("locationActions.copyFailed"));
    }
  };

  const reveal = async () => {
    if (absolutePath === null) {
      toast.error(t("locationActions.pathUnavailable"));
      return;
    }
    try {
      await locationActions.open("explorer", absolutePath);
    } catch {
      toast.error(
        t("locationActions.openFailed", {
          app: t("locationActions.explorer"),
        }),
      );
    }
  };

  return (
    <ContextMenu>
      <ContextMenuTrigger
        render={
          <div
            className="w-full"
            onContextMenu={(event) => event.preventDefault()}
          />
        }
      >
        {children}
      </ContextMenuTrigger>
      <ContextMenuContent
        className="w-52"
        onClick={(event) => event.stopPropagation()}
      >
        <ContextMenuItem onClick={onNewFile}>
          <IconFilePlus />
          {t("files.newFile")}
        </ContextMenuItem>
        <ContextMenuItem onClick={onNewFolder}>
          <IconFolderPlus />
          {t("files.newFolder")}
        </ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem disabled={!canEditEntry} onClick={onCut}>
          <IconCut />
          {t("files.cut")}
        </ContextMenuItem>
        <ContextMenuItem disabled={!canEditEntry} onClick={onCopy}>
          <IconCopy />
          {t("files.copy")}
        </ContextMenuItem>
        <ContextMenuItem disabled={!canPaste} onClick={onPaste}>
          <IconClipboard />
          {t("files.paste")}
        </ContextMenuItem>
        <ContextMenuItem disabled={!canEditEntry} onClick={onRename}>
          <IconPencil />
          {t("files.rename")}
        </ContextMenuItem>
        {onDelete !== undefined && (
          <ContextMenuItem
            variant="destructive"
            disabled={!canEditEntry}
            onClick={onDelete}
          >
            <IconTrash />
            {t("common.delete")}
          </ContextMenuItem>
        )}
        <ContextMenuSeparator />
        <ContextMenuItem
          disabled={absolutePath === null}
          onClick={() => {
            if (absolutePath !== null) void copyText(absolutePath);
          }}
        >
          <IconCopy />
          {t("files.copyPath")}
        </ContextMenuItem>
        <ContextMenuItem onClick={() => void copyText(relativeLabel)}>
          <IconCopy />
          {t("files.copyRelativePath")}
        </ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem
          disabled={absolutePath === null}
          onClick={() => void reveal()}
        >
          <IconFolderShare />
          {t("files.revealInFileManager")}
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  );
}

/** Inline name field used after New File / New Folder / Rename. */
function NameDraftRow({
  depth,
  kind,
  initialName,
  ariaLabel,
  onCommit,
  onCancel,
}: {
  depth: number;
  kind: "file" | "directory";
  initialName: string;
  ariaLabel: string;
  onCommit: (name: string) => Promise<void>;
  onCancel: () => void;
}) {
  const { t } = useTranslation();
  const showContractError = useContractErrorToast();
  const [draft, setDraft] = useState(initialName);
  const inputRef = useRef<HTMLInputElement>(null);
  const skipBlurCommit = useRef(true);
  const committingRef = useRef(false);

  useEffect(() => {
    inputRef.current?.focus();
    inputRef.current?.select();
    const settle = window.setTimeout(() => {
      skipBlurCommit.current = false;
    }, 0);
    return () => window.clearTimeout(settle);
  }, []);

  const commit = async () => {
    if (committingRef.current) return;
    const name = draft.trim();
    if (name === "" || name === initialName) {
      onCancel();
      return;
    }
    if (
      name.includes("/") ||
      name.includes("\\") ||
      name === "." ||
      name === ".."
    ) {
      toast.error(t("files.invalidEntryName"));
      inputRef.current?.focus();
      return;
    }
    committingRef.current = true;
    try {
      await onCommit(name);
    } catch (cause) {
      showContractError(cause);
      inputRef.current?.focus();
    } finally {
      committingRef.current = false;
    }
  };

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.nativeEvent.isComposing) return;
    if (event.key === "Enter") {
      event.preventDefault();
      void commit();
    } else if (event.key === "Escape") {
      event.preventDefault();
      skipBlurCommit.current = true;
      onCancel();
    }
  };

  return (
    <div
      className="flex h-7 items-center gap-1 pr-2"
      style={{ paddingLeft: `${8 + depth * 14}px` }}
    >
      {kind === "directory" ? (
        <IconFolder className="size-4 shrink-0 text-amber-600" />
      ) : (
        <span className="w-4" />
      )}
      <Input
        ref={inputRef}
        value={draft}
        aria-label={ariaLabel}
        className="h-6 flex-1 border-transparent bg-background px-1.5 text-xs shadow-none"
        onChange={(event) => setDraft(event.target.value)}
        onClick={(event) => event.stopPropagation()}
        onKeyDown={onKeyDown}
        onBlur={() => {
          if (skipBlurCommit.current) {
            skipBlurCommit.current = false;
            return;
          }
          void commit();
        }}
      />
    </div>
  );
}
