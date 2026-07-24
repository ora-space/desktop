import type { FileSystemEntry, ListDirectoryResponse } from "@ora/contracts";
import {
  Button,
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Input,
  Label,
  Spinner,
  cn,
} from "@ora/ui";
import {
  IconArrowUp,
  IconFile,
  IconFolder,
  IconFolderSymlink,
  IconLinkOff,
  IconRefresh,
} from "@tabler/icons-react";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
  useCallback,
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type FormEvent,
  type KeyboardEvent,
} from "react";
import type { PlatformLocale, SelectPathOptions } from "../types";
import { platformMessages } from "./messages";
import type { WebPlatformAdapter } from "./web-platform-adapter";

const ENTRY_ROW_HEIGHT = 36;
const PATH_PICKER_GUTTER = "1.5rem";
const EMPTY_ENTRIES: FileSystemEntry[] = [];

/** Renders the active Web selection request and stays absent for an idle adapter. */
export function WebPathPickerHost({
  adapter,
  locale,
}: {
  adapter: WebPlatformAdapter;
  locale: PlatformLocale;
}) {
  const snapshot = useSyncExternalStore(adapter.subscribe, adapter.getSnapshot, adapter.getSnapshot);
  if (snapshot.kind === "idle") {
    return null;
  }

  return (
    <WebPathPickerDialog
      key={snapshot.requestId}
      adapter={adapter}
      locale={locale}
      options={snapshot.options}
      requestId={snapshot.requestId}
    />
  );
}

interface WebPathPickerDialogProps {
  adapter: WebPlatformAdapter;
  locale: PlatformLocale;
  options: SelectPathOptions;
  requestId: number;
}

/** Provides the Web implementation of the single file-or-directory selection interaction. */
function WebPathPickerDialog({
  adapter,
  locale,
  options,
  requestId,
}: WebPathPickerDialogProps) {
  const messages = platformMessages(locale);
  const [directory, setDirectory] = useState<ListDirectoryResponse | null>(null);
  const [pathDraft, setPathDraft] = useState(options.initialPath ?? "");
  const [selectedIndex, setSelectedIndex] = useState(-1);
  const [loading, setLoading] = useState(true);
  const [readError, setReadError] = useState(false);
  const loadSequence = useRef(0);
  const listRef = useRef<HTMLDivElement>(null);

  const applyDirectory = useCallback((response: ListDirectoryResponse) => {
    // Each directory is a distinct view. Keeping the prior scroll offset can make
    // a short destination appear blank after navigating from a long one.
    if (listRef.current !== null) {
      listRef.current.scrollTop = 0;
    }
    setDirectory(response);
    setPathDraft(response.currentPath);
    setSelectedIndex(-1);
    setReadError(false);
  }, []);

  const loadDirectory = useCallback(
    async (path: string | undefined) => {
      const sequence = loadSequence.current + 1;
      loadSequence.current = sequence;
      if (path !== undefined) {
        setPathDraft(path);
      }
      setSelectedIndex(-1);
      setLoading(true);
      setReadError(false);
      try {
        const response = await adapter.client.fileSystem.listDirectory(
          path === undefined ? {} : { path },
        );
        if (loadSequence.current === sequence) {
          applyDirectory(response);
        }
      } catch {
        if (loadSequence.current === sequence) {
          setReadError(true);
        }
      } finally {
        if (loadSequence.current === sequence) {
          setLoading(false);
        }
      }
    },
    [adapter.client, applyDirectory],
  );

  useEffect(() => {
    let active = true;
    const sequence = loadSequence.current + 1;
    loadSequence.current = sequence;

    /** Falls back to home only for an invalid initial path; home failures remain visible for retry. */
    async function loadInitialDirectory() {
      setLoading(true);
      try {
        let response: ListDirectoryResponse;
        if (options.initialPath === undefined) {
          response = await adapter.client.fileSystem.listDirectory({});
        } else {
          try {
            response = await adapter.client.fileSystem.listDirectory({ path: options.initialPath });
          } catch {
            response = await adapter.client.fileSystem.listDirectory({});
          }
        }

        if (active && loadSequence.current === sequence) {
          applyDirectory(response);
        }
      } catch {
        if (active && loadSequence.current === sequence) {
          setReadError(true);
        }
      } finally {
        if (active && loadSequence.current === sequence) {
          setLoading(false);
        }
      }
    }

    void loadInitialDirectory();
    return () => {
      active = false;
    };
  }, [adapter.client, applyDirectory, options.initialPath]);

  const entries = directory?.entries ?? EMPTY_ENTRIES;
  const selectedEntry = selectedIndex === -1 ? undefined : entries[selectedIndex];
  const confirmableEntry =
    selectedEntry !== undefined && selectedEntry.kind === options.kind ? selectedEntry : undefined;
  const getItemKey = useCallback(
    (index: number) => entries[index]?.path ?? index,
    [entries],
  );
  // TanStack Virtual intentionally owns mutable scroll state outside React memoization.
  // eslint-disable-next-line react-hooks/incompatible-library
  const virtualizer = useVirtualizer({
    count: entries.length,
    getScrollElement: () => listRef.current,
    estimateSize: () => ENTRY_ROW_HEIGHT,
    getItemKey,
    overscan: 8,
    initialRect: { width: 640, height: 288 },
    enabled: true,
  });

  const navigateTo = useCallback(
    (path: string) => {
      void loadDirectory(path);
    },
    [loadDirectory],
  );

  const activateEntry = useCallback(
    (entry: FileSystemEntry) => {
      if (entry.kind === "directory") {
        navigateTo(entry.path);
      } else if (entry.kind === "file" && options.kind === "file") {
        adapter.completeSelection(requestId, entry.path);
      }
    },
    [adapter, navigateTo, options.kind, requestId],
  );

  const handleListKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === "Escape") {
      event.preventDefault();
      adapter.completeSelection(requestId, null);
      return;
    }
    if (event.key === "Backspace" && directory?.parentPath !== null && directory?.parentPath !== undefined) {
      event.preventDefault();
      navigateTo(directory.parentPath);
      return;
    }
    if (entries.length === 0) {
      return;
    }
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      const delta = event.key === "ArrowDown" ? 1 : -1;
      const nextIndex = selectedIndex === -1
        ? 0
        : Math.max(0, Math.min(entries.length - 1, selectedIndex + delta));
      setSelectedIndex(nextIndex);
      virtualizer.scrollToIndex(nextIndex, { align: "auto" });
      return;
    }
    if (event.key === "Enter" && !loading && !readError && selectedEntry !== undefined) {
      event.preventDefault();
      activateEntry(selectedEntry);
    }
  };

  const handlePathSubmit = (event: FormEvent) => {
    event.preventDefault();
    const path = pathDraft.trim();
    if (path !== "") {
      void loadDirectory(path);
    }
  };

  const title =
    options.kind === "directory" ? messages.chooseDirectoryTitle : messages.chooseFileTitle;
  // Read the mutable virtual range on every render so scroll-triggered renders cannot reuse stale rows.
  const positionedItems = virtualizer
    .getVirtualItems()
    .map((item) => ({ index: item.index, start: item.start }));
  const listHeight = virtualizer.getTotalSize();
  const activeDescendant = selectedIndex === -1 ? undefined : `platform-path-entry-${selectedIndex}`;

  return (
    <Dialog open onOpenChange={(open) => !open && adapter.completeSelection(requestId, null)}>
      <DialogContent
        className="min-h-0 min-w-0 max-w-none gap-0 overflow-hidden rounded-lg! p-0 sm:max-w-none"
        showCloseButton={false}
        style={{
          display: "flex",
          flexDirection: "column",
          inlineSize: "40rem",
          blockSize: "30rem",
          maxInlineSize: "calc(100vw - 2rem)",
          maxBlockSize: "calc(100dvh - 2rem)",
        }}
      >
        <DialogHeader
          className="min-w-0 shrink-0 gap-1"
          style={{ padding: `${PATH_PICKER_GUTTER} ${PATH_PICKER_GUTTER} 0.75rem` }}
        >
          <DialogTitle>{title}</DialogTitle>
        </DialogHeader>

        <form
          className="mb-3 flex shrink-0 items-center gap-1 rounded-md bg-muted/60 p-1"
          style={{ marginInline: PATH_PICKER_GUTTER }}
          onSubmit={handlePathSubmit}
        >
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            className="shrink-0"
            disabled={loading || directory?.parentPath == null}
            onClick={() => directory?.parentPath != null && navigateTo(directory.parentPath)}
            aria-label={messages.up}
            title={messages.up}
          >
            <IconArrowUp />
          </Button>
          <div className="min-w-0 flex-1">
            <Label htmlFor="platform-path-input" className="sr-only">
              {messages.pathLabel}
            </Label>
            <Input
              id="platform-path-input"
              aria-label={messages.pathLabel}
              value={pathDraft}
              onChange={(event) => setPathDraft(event.target.value)}
              className="border-transparent bg-background/70 shadow-none focus-visible:border-transparent focus-visible:ring-0"
            />
          </div>
          <Button type="submit" variant="ghost" className="w-16 shrink-0" disabled={loading || pathDraft.trim() === ""}>
            {loading && <Spinner className="size-3.5" aria-hidden="true" />}
            {messages.go}
          </Button>
        </form>

        <div
          ref={listRef}
          role="listbox"
          tabIndex={0}
          aria-activedescendant={activeDescendant}
          aria-label={title}
          aria-busy={loading}
          className="relative mb-3 min-h-0 flex-1 overflow-auto rounded-md bg-muted/45 outline-none focus-visible:ring-2 focus-visible:ring-ring"
          style={{ marginInline: PATH_PICKER_GUTTER }}
          onKeyDown={handleListKeyDown}
        >
          {!loading && readError && (
            <div role="alert" className="grid h-full place-items-center gap-3 p-6 text-center">
              <p data-selectable className="text-sm text-destructive">{messages.readError}</p>
              <Button type="button" variant="outline" onClick={() => void loadDirectory(pathDraft.trim() || undefined)}>
                <IconRefresh />
                {messages.retry}
              </Button>
            </div>
          )}
          {!loading && !readError && entries.length === 0 && (
            <div className="grid h-full place-items-center text-sm text-muted-foreground">
              {messages.emptyDirectory}
            </div>
          )}
          {!readError && entries.length > 0 && (
            <div className="relative w-full" style={{ height: listHeight }}>
              {positionedItems.map((virtualItem) => {
                const entry = entries[virtualItem.index]!;
                const selected = virtualItem.index === selectedIndex;
                return (
                  <div
                    id={`platform-path-entry-${virtualItem.index}`}
                    key={entry.path}
                    role="option"
                    aria-selected={selected}
                    aria-disabled={entry.kind === "unavailable"}
                    className={cn(
                      "absolute left-0 top-0 flex h-9 w-full min-w-0 items-center gap-2 rounded-md px-3 text-sm transition-colors duration-150 hover:bg-background/70",
                      selected && "bg-background text-foreground shadow-sm",
                      entry.kind === "unavailable" && "text-muted-foreground opacity-60",
                    )}
                    style={{ transform: `translateY(${virtualItem.start}px)` }}
                    onClick={() => entry.kind !== "unavailable" && setSelectedIndex(virtualItem.index)}
                    onDoubleClick={() => entry.kind !== "unavailable" && activateEntry(entry)}
                  >
                    <EntryIcon entry={entry} />
                    <span className="min-w-0 flex-1 truncate">{entry.name}</span>
                  </div>
                );
              })}
            </div>
          )}
          {loading && (
            <div className="absolute inset-0 grid place-items-center bg-muted/55" aria-live="polite">
              <span className="flex items-center gap-2 text-xs text-muted-foreground">
                <Spinner className="size-3.5" />
                {messages.loading}
              </span>
            </div>
          )}
        </div>

        <DialogFooter
          className="mx-0 mb-0 shrink-0 border-t-0 bg-transparent pt-0"
          style={{ paddingInline: PATH_PICKER_GUTTER, paddingBottom: PATH_PICKER_GUTTER }}
        >
          <Button type="button" variant="outline" onClick={() => adapter.completeSelection(requestId, null)}>
            {messages.cancel}
          </Button>
          {options.kind === "directory" && (
            <Button
              type="button"
              variant="secondary"
              disabled={loading || readError || directory === null}
              onClick={() => directory !== null && adapter.completeSelection(requestId, directory.currentPath)}
            >
              {messages.chooseCurrentDirectory}
            </Button>
          )}
          <Button
            type="button"
            disabled={loading || readError || confirmableEntry === undefined}
            onClick={() => confirmableEntry !== undefined && adapter.completeSelection(requestId, confirmableEntry.path)}
          >
            {messages.choose}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/** Chooses an icon that communicates entry type and symbolic-link state without changing selection rules. */
function EntryIcon({ entry }: { entry: FileSystemEntry }) {
  if (entry.kind === "unavailable") {
    return <IconLinkOff className="size-4 shrink-0" />;
  }
  if (entry.kind === "directory" && entry.isSymbolicLink) {
    return <IconFolderSymlink className="size-4 shrink-0" />;
  }
  if (entry.kind === "directory") {
    return <IconFolder className="size-4 shrink-0" />;
  }

  return <IconFile className="size-4 shrink-0" />;
}
