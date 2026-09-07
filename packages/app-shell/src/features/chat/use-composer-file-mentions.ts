import { useEffect, useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { fileKeys } from "../../state/data/files";
import {
  type ComposerMentionEntry,
  mentionEntriesFromDirectoryListing,
  mentionEntriesFromFileSearch,
  rankComposerMentionEntries,
} from "./composer-actions";

/** Matches the explorer filter debounce so @-search feels the same as the sidebar. */
export const FILE_MENTION_DEBOUNCE_MS = 200;

export type ComposerFileMentionStatus =
  "inactive" | "need-project" | "loading" | "error" | "empty" | "ready";

export type ComposerFileMentionMenuStatus =
  "ready" | "loading" | "empty" | "error";

export interface ComposerFileMentionState {
  /** True while the @ palette should own the action menu chrome. */
  active: boolean;
  entries: ComposerMentionEntry[];
  status: ComposerFileMentionStatus;
  /**
   * True while the typed query has not settled or a request is in flight.
   * Callers should keep rows visible when possible but refuse selection.
   */
  selectionLocked: boolean;
  truncated: boolean;
  /** Debounced query string used for search; `""` means root listing. */
  debouncedQuery: string | null;
}

/** Maps mention status onto the shared action-menu status chrome. */
export function fileMentionMenuStatus(
  status: ComposerFileMentionStatus,
): ComposerFileMentionMenuStatus {
  switch (status) {
    case "loading":
      return "loading";
    case "error":
      return "error";
    case "need-project":
    case "empty":
      return "empty";
    case "ready":
    case "inactive":
      return "ready";
  }
}

/** i18n key for the @ file palette status row, if any. */
export function fileMentionStatusMessageKey(
  status: ComposerFileMentionStatus,
  debouncedQuery: string | null,
): string | undefined {
  switch (status) {
    case "need-project":
      return "chat.actionMenu.filesNeedProject";
    case "loading":
      return "chat.actionMenu.filesSearching";
    case "error":
      return "chat.actionMenu.filesError";
    case "empty":
      return debouncedQuery === ""
        ? "chat.actionMenu.filesTypeToSearch"
        : "chat.actionMenu.filesEmpty";
    case "ready":
    case "inactive":
      return undefined;
  }
}

type ResolvedEntries = ComposerMentionEntry[] | "keep" | "clear";

/**
 * Loads workspace file and folder candidates for the composer `@` palette.
 *
 * Prefers the task worktree when `taskId` is set; otherwise searches the
 * project checkout so draft chats can mention paths before a task exists.
 *
 * Debounce and in-flight requests keep the previous hit list visible
 * (`selectionLocked`) so the menu does not flash empty between queries.
 */
export function useComposerFileMentions({
  taskId,
  projectId,
  atQuery,
  enabled,
}: {
  taskId: string | undefined;
  projectId: string | undefined;
  atQuery: string | null;
  enabled: boolean;
}): ComposerFileMentionState {
  const client = useContractsClient();
  const active = enabled && atQuery !== null;
  const [debouncedAtQuery, setDebouncedAtQuery] = useState<string | null>(null);
  // Retains the last successful hit list across debounce / in-flight gaps.
  const [retainedEntries, setRetainedEntries] = useState<
    ComposerMentionEntry[]
  >([]);

  // Clear immediately when the palette closes so the next open cannot briefly
  // search a stale debounced string from the previous session.
  if ((!enabled || atQuery === null) && debouncedAtQuery !== null) {
    setDebouncedAtQuery(null);
  }

  useEffect(() => {
    if (atQuery === null || !enabled) return;
    const timer = window.setTimeout(() => {
      setDebouncedAtQuery(atQuery);
    }, FILE_MENTION_DEBOUNCE_MS);
    return () => window.clearTimeout(timer);
  }, [atQuery, enabled]);

  const querySettled =
    atQuery !== null &&
    debouncedAtQuery !== null &&
    debouncedAtQuery === atQuery;
  const scopeKind =
    taskId !== undefined ? "task" : projectId !== undefined ? "project" : null;
  const scopeId = taskId ?? projectId;
  const hasScope = scopeKind !== null && scopeId !== undefined;
  const rootListingActive = active && hasScope && debouncedAtQuery === "";
  const fileSearchActive =
    active &&
    hasScope &&
    debouncedAtQuery !== null &&
    debouncedAtQuery.length > 0;

  const rootFilesQuery = useQuery({
    queryKey:
      scopeKind === "task"
        ? fileKeys.workspaceDirectory(scopeId ?? "", "")
        : fileKeys.projectDirectory(scopeId ?? "", ""),
    queryFn: ({ signal }) => {
      if (scopeKind === "task") {
        return client.fileSystem.listWorkspaceDirectory(
          { taskId: scopeId!, path: "" },
          { signal },
        );
      }
      return client.fileSystem.listProjectDirectory(
        { projectId: scopeId!, path: "" },
        { signal },
      );
    },
    // Stay subscribed while debounce still holds the previous empty query so
    // root hits remain on screen until the next settled search starts.
    enabled: rootListingActive,
  });
  const fileSearchQuery = useQuery({
    queryKey:
      scopeKind === "task"
        ? fileKeys.workspaceSearch(
            scopeId ?? "",
            "files",
            debouncedAtQuery ?? "",
          )
        : fileKeys.projectSearch(
            scopeId ?? "",
            "files",
            debouncedAtQuery ?? "",
          ),
    queryFn: ({ signal }) => {
      if (scopeKind === "task") {
        return client.fileSystem.searchWorkspace(
          {
            taskId: scopeId!,
            query: debouncedAtQuery!,
            kind: "files",
          },
          { signal },
        );
      }
      return client.fileSystem.searchProject(
        {
          projectId: scopeId!,
          query: debouncedAtQuery!,
          kind: "files",
        },
        { signal },
      );
    },
    enabled: fileSearchActive,
  });

  const awaitingDebounce =
    active && hasScope && atQuery !== null && !querySettled;
  const rootBusy =
    rootListingActive && querySettled && rootFilesQuery.isPending;
  const searchBusy =
    fileSearchActive && querySettled && fileSearchQuery.isPending;
  /** True while the settled query still has no payload (not background refetch). */
  const fetching = rootBusy || searchBusy;
  const selectionLocked = awaitingDebounce || fetching;

  const rootError = rootListingActive && querySettled && rootFilesQuery.isError;
  const searchError =
    fileSearchActive && querySettled && fileSearchQuery.isError;
  const errored = rootError || searchError;

  const resolvedEntries = useMemo((): ResolvedEntries => {
    if (!active || !hasScope || errored) return "clear";
    if (debouncedAtQuery === null) return "keep";

    if (debouncedAtQuery === "") {
      if (rootFilesQuery.data === undefined) return "keep";
      return rankComposerMentionEntries(
        mentionEntriesFromDirectoryListing(rootFilesQuery.data.entries),
        "",
      );
    }

    if (fileSearchQuery.data === undefined) return "keep";
    return rankComposerMentionEntries(
      mentionEntriesFromFileSearch(
        fileSearchQuery.data.results,
        debouncedAtQuery,
      ),
      debouncedAtQuery,
    );
  }, [
    active,
    debouncedAtQuery,
    errored,
    fileSearchQuery.data,
    hasScope,
    rootFilesQuery.data,
  ]);

  // Render-phase retention: keep prior hits while debounce / fetch replaces them.
  if (resolvedEntries === "clear") {
    if (retainedEntries.length > 0) setRetainedEntries([]);
  } else if (
    resolvedEntries !== "keep" &&
    resolvedEntries !== retainedEntries
  ) {
    setRetainedEntries(resolvedEntries);
  }

  const entries =
    resolvedEntries === "clear"
      ? []
      : resolvedEntries === "keep"
        ? retainedEntries
        : resolvedEntries;

  const truncated =
    active &&
    !fetching &&
    !errored &&
    debouncedAtQuery !== null &&
    debouncedAtQuery.length > 0 &&
    (fileSearchQuery.data?.truncated ?? false);

  let status: ComposerFileMentionStatus = "inactive";
  if (active) {
    if (scopeKind === null) {
      status = "need-project";
    } else if (fetching || (awaitingDebounce && entries.length === 0)) {
      // First open / no prior hits: searching chrome. Debounce with prior hits
      // keeps rows without the spinner; a live fetch still shows loading.
      status = "loading";
    } else if (errored) {
      status = "error";
    } else if (entries.length === 0) {
      status = "empty";
    } else {
      status = "ready";
    }
  }

  return {
    active,
    entries,
    status,
    selectionLocked,
    truncated,
    debouncedQuery: active ? debouncedAtQuery : null,
  };
}
