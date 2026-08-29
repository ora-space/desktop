import type * as acp from "@agentclientprotocol/sdk";
import type { Agent, Skill } from "@ora/contracts";
import type { PluginEntry } from "../settings/plugin-catalog";
import { availableSkills } from "../../state/hooks/use-skills";

export type ComposerActionGroup =
  "files" | "skills" | "commands" | "roles" | "plugins" | "actions";

export type ComposerAction =
  | {
      id: string;
      group: "files";
      label: string;
      description: string;
      path: string;
      /** Folder mentions share the path-chip payload; kind drives the glyph. */
      entryKind: "file" | "directory";
    }
  | {
      id: string;
      group: "skills";
      label: string;
      description: string;
      skill: Skill;
    }
  | {
      id: string;
      group: "commands";
      label: string;
      description: string;
      hint?: string;
      command: acp.AvailableCommand;
    }
  | {
      id: string;
      group: "roles";
      label: string;
      description: string;
      role: Agent;
    }
  | {
      id: string;
      group: "plugins";
      label: string;
      description: string;
      plugin: PluginEntry;
    }
  | {
      id: "action:add-images";
      group: "actions";
      label: string;
      description: string;
    };

export const COMPOSER_ACTION_GROUPS: readonly ComposerActionGroup[] = [
  "files",
  "skills",
  "commands",
  "roles",
  "plugins",
  "actions",
];
export const COLLAPSED_ACTION_GROUP_SIZE = 5;
/** Caps @-mention file rows so the palette stays compact and scrollable. */
export const MAX_COMPOSER_FILE_ACTIONS = 12;

/** Builds searchable actions from provider capabilities, Ora's configured skills and roles, and the plugin catalog. */
export function buildComposerActions({
  skills,
  commands,
  roles,
  plugins,
  translatePluginSummary,
  includeAttachments,
  attachmentLabel,
  attachmentDescription,
}: {
  skills: Skill[];
  commands: acp.AvailableCommand[];
  roles: Agent[];
  plugins: PluginEntry[];
  translatePluginSummary: (summaryKey: string) => string;
  includeAttachments: boolean;
  attachmentLabel: string;
  attachmentDescription: string;
}): ComposerAction[] {
  return [
    ...availableSkills(skills).map((skill): ComposerAction => ({
      id: `skill:${skill.id}`,
      group: "skills",
      label: skill.name,
      description: skill.description,
      skill,
    })),
    ...commands.map((command): ComposerAction => ({
      id: `command:${command.name}`,
      group: "commands",
      label: command.name,
      description: command.description,
      ...(command.input == null ? {} : { hint: command.input.hint }),
      command,
    })),
    ...roles.map((role): ComposerAction => ({
      id: `role:${role.id}`,
      group: "roles",
      label: role.name,
      description: role.description,
      role,
    })),
    ...plugins.map((plugin): ComposerAction => ({
      id: `plugin:${plugin.id}`,
      group: "plugins",
      label: plugin.name,
      description: translatePluginSummary(plugin.summaryKey),
      plugin,
    })),
    ...(includeAttachments
      ? [
          {
            id: "action:add-images" as const,
            group: "actions" as const,
            label: attachmentLabel,
            description: attachmentDescription,
          },
        ]
      : []),
  ];
}

/** One @-palette row: a workspace file or directory path. */
export type ComposerMentionEntry = {
  path: string;
  kind: "file" | "directory";
};

/**
 * Caps mention lists early so a large ripgrep payload never becomes a full
 * in-memory action table before the menu limit applies.
 */
export function takeComposerMentionEntries(
  entries: Iterable<ComposerMentionEntry>,
  limit: number = MAX_COMPOSER_FILE_ACTIONS,
): ComposerMentionEntry[] {
  const unique: ComposerMentionEntry[] = [];
  const seen = new Set<string>();
  for (const entry of entries) {
    if (seen.has(entry.path)) continue;
    seen.add(entry.path);
    unique.push(entry);
    if (unique.length >= limit) break;
  }
  return unique;
}

/**
 * Dedupes the full candidate set, ranks for a pleasant picker order, then
 * caps to the menu size. Scoring must run before any pool cut so late
 * walk-order hits with a strong basename match still surface.
 */
export function rankComposerMentionEntries(
  entries: Iterable<ComposerMentionEntry>,
  query: string,
  limit: number = MAX_COMPOSER_FILE_ACTIONS,
): ComposerMentionEntry[] {
  const unique: ComposerMentionEntry[] = [];
  const seen = new Set<string>();
  for (const entry of entries) {
    if (seen.has(entry.path)) continue;
    seen.add(entry.path);
    unique.push(entry);
  }
  unique.sort((left, right) =>
    compareComposerMentionEntries(left, right, query),
  );
  return unique.slice(0, limit);
}

/**
 * Sort comparator for @ hits. Higher relevance first; stable letter order as
 * the everyday tie-breaker so the list does not feel like ripgrep walk order.
 */
export function compareComposerMentionEntries(
  left: ComposerMentionEntry,
  right: ComposerMentionEntry,
  query: string,
): number {
  const normalizedQuery = query.trim().toLocaleLowerCase();
  if (normalizedQuery !== "") {
    const scoreDiff =
      mentionRelevanceScore(right, normalizedQuery) -
      mentionRelevanceScore(left, normalizedQuery);
    if (scoreDiff !== 0) return scoreDiff;
  }

  const kindDiff =
    (left.kind === "directory" ? 0 : 1) - (right.kind === "directory" ? 0 : 1);
  if (kindDiff !== 0) return kindDiff;

  const nameDiff = fileBasename(left.path).localeCompare(
    fileBasename(right.path),
    undefined,
    { sensitivity: "base" },
  );
  if (nameDiff !== 0) return nameDiff;
  return left.path.localeCompare(right.path, undefined, {
    sensitivity: "base",
  });
}

/**
 * Soft relevance buckets: exact / prefix / basename / path. Kept shallow so a
 * path substring match cannot outrank a clear basename hit.
 */
export function mentionRelevanceScore(
  entry: ComposerMentionEntry,
  normalizedQuery: string,
): number {
  const base = fileBasename(entry.path).toLocaleLowerCase();
  const path = entry.path.replace(/\\/g, "/").toLocaleLowerCase();
  const query = normalizedQuery.replace(/\\/g, "/");
  if (base === query) return 100;
  if (base.startsWith(query)) return 80;
  if (base.includes(query)) return 60;
  if (
    path.startsWith(`${query}/`) ||
    path.includes(`/${query}/`) ||
    path.endsWith(`/${query}`)
  ) {
    return 40;
  }
  if (path.includes(query)) return 20;
  return 0;
}

/**
 * Caps path lists early. Prefer {@link takeComposerMentionEntries} when kind
 * matters; this keeps string-only call sites and tests concise.
 */
export function takeComposerFilePaths(
  paths: Iterable<string>,
  limit: number = MAX_COMPOSER_FILE_ACTIONS,
): string[] {
  return takeComposerMentionEntries(
    (function* () {
      for (const path of paths) yield { path, kind: "file" as const };
    })(),
    limit,
  ).map((entry) => entry.path);
}

/**
 * Turns workspace mention entries into @-menu actions. Label is the basename;
 * description is the parent directory (or `.` at the repo root).
 */
export function buildComposerFileActions(
  entries: readonly ComposerMentionEntry[],
): ComposerAction[] {
  return takeComposerMentionEntries(entries).map((entry): ComposerAction => ({
    id: `file:${entry.path}`,
    group: "files",
    label: fileBasename(entry.path),
    description: fileParentPath(entry.path),
    path: entry.path,
    entryKind: entry.kind,
  }));
}

/**
 * Root `@` listing rows. Final order comes from {@link rankComposerMentionEntries}.
 */
export function* mentionEntriesFromDirectoryListing(
  entries: readonly { path: string; kind: "file" | "directory" }[] | undefined,
): Iterable<ComposerMentionEntry> {
  if (entries === undefined) return;
  for (const entry of entries) {
    if (entry.kind === "directory" || entry.kind === "file") {
      yield { path: entry.path, kind: entry.kind };
    }
  }
}

/**
 * Search hits only list files (`rg --files`). Matching ancestor directories are
 * added so `@src` can still pick a folder; ranking decides display order.
 */
export function* mentionEntriesFromFileSearch(
  results: readonly { kind: string; path: string }[] | undefined,
  query: string,
): Iterable<ComposerMentionEntry> {
  if (results === undefined) return;
  const normalizedQuery = query.trim().toLocaleLowerCase();
  const seenDirs = new Set<string>();

  for (const result of results) {
    if (result.kind !== "file") continue;
    yield { path: result.path, kind: "file" };
    for (const directory of ancestorDirectories(result.path)) {
      if (seenDirs.has(directory)) continue;
      if (
        normalizedQuery !== "" &&
        !directory.toLocaleLowerCase().includes(normalizedQuery)
      ) {
        continue;
      }
      seenDirs.add(directory);
      yield { path: directory, kind: "directory" };
    }
  }
}

/** Slash-separated parents of a workspace-relative file path, root-first. */
export function ancestorDirectories(path: string): string[] {
  const normalized = path.replace(/\\/g, "/");
  const parts = normalized.split("/").filter((part) => part.length > 0);
  if (parts.length <= 1) return [];
  const directories: string[] = [];
  for (let index = 1; index < parts.length; index += 1) {
    directories.push(parts.slice(0, index).join("/"));
  }
  return directories;
}

/** Filters actions with one predictable name-and-description search rule. */
export function filterComposerActions(
  actions: ComposerAction[],
  query: string,
): ComposerAction[] {
  const normalizedQuery = query.trim().toLocaleLowerCase();
  if (normalizedQuery === "") return actions;
  return actions.filter(
    (action) =>
      action.label.toLocaleLowerCase().includes(normalizedQuery) ||
      action.description.toLocaleLowerCase().includes(normalizedQuery) ||
      (action.group === "files" &&
        action.path.toLocaleLowerCase().includes(normalizedQuery)),
  );
}

/** Limits long capability groups until the user explicitly asks to reveal them. */
export function visibleComposerActions(
  actions: ComposerAction[],
  expandedGroups: ReadonlySet<ComposerActionGroup>,
): ComposerAction[] {
  return COMPOSER_ACTION_GROUPS.flatMap((group) => {
    const groupActions = actions.filter((action) => action.group === group);
    // File mentions are a flat scrollable list; collapsing them forces an extra
    // click that Cursor-style @ pickers do not require.
    if (group === "files" || expandedGroups.has(group)) {
      return groupActions;
    }
    return groupActions.slice(0, COLLAPSED_ACTION_GROUP_SIZE);
  });
}

/** Basename of a slash- or backslash-separated workspace path. */
export function fileBasename(path: string): string {
  const parts = path.split(/[/\\]/);
  return parts[parts.length - 1] || path;
}

/** Parent directory for subtitle display; `.` when the file sits at the root. */
export function fileParentPath(path: string): string {
  const normalized = path.replace(/\\/g, "/");
  const slash = normalized.lastIndexOf("/");
  if (slash <= 0) return ".";
  return normalized.slice(0, slash);
}
