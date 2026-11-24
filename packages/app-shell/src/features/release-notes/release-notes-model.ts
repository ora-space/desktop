import type { ReleaseNote } from "../../lib/release-note";

/** Parses the stable part of a version, ignoring prerelease and build suffixes. */
function parseStableVersion(version: string): number[] | null {
  const match = /^(\d+)\.(\d+)\.(\d+)(?:[-+].*)?$/.exec(version);
  return match === null ? null : match.slice(1).map(Number);
}

/** Returns the stable version whose notes a build shows, or null when the version is malformed. */
export function stableVersionOf(version: string): string | null {
  return parseStableVersion(version)?.join(".") ?? null;
}

/** Orders two stable versions numerically; both must already be well formed. */
function compareVersions(left: string, right: string): number {
  const rightParts = parseStableVersion(right) ?? [];
  for (const [index, part] of (parseStableVersion(left) ?? []).entries()) {
    const difference = part - (rightParts[index] ?? 0);
    if (difference !== 0) return difference;
  }
  return 0;
}

/** Lists the notes a build may show: never newer than itself, newest first. */
export function visibleReleaseNotes(
  notes: readonly ReleaseNote[],
  appVersion: string,
): ReleaseNote[] {
  const current = stableVersionOf(appVersion);
  if (current === null) return [];
  return notes
    .filter(
      (note) =>
        stableVersionOf(note.version) !== null &&
        compareVersions(note.version, current) <= 0,
    )
    .sort((left, right) => compareVersions(right.version, left.version));
}

/** What a start should do: which notes to prompt, and which version to remember as prompted. */
export interface ReleaseNotesPrompt {
  version: string;
  notes: ReleaseNote[];
}

/**
 * Decides whether this start has unseen notes.
 *
 * Returns null when nothing may be shown or remembered: the build has no notes of its own version, so
 * its version says nothing about releases, or the version was already prompted. An empty list means
 * the version is remembered without prompting, which is how a fresh installation starts.
 */
export function planReleaseNotesPrompt({
  notes,
  appVersion,
  promptedVersion,
  newInstallation,
}: {
  notes: readonly ReleaseNote[];
  appVersion: string;
  /** The highest version already prompted on this machine, or null without a usable record. */
  promptedVersion: string | null;
  newInstallation: boolean;
}): ReleaseNotesPrompt | null {
  const version = stableVersionOf(appVersion);
  if (version === null) return null;
  const visible = visibleReleaseNotes(notes, appVersion);
  if (!visible.some((note) => note.version === version)) return null;
  if (promptedVersion === null) {
    return { version, notes: newInstallation ? [] : visible };
  }
  if (compareVersions(version, promptedVersion) <= 0) return null;
  return {
    version,
    notes: visible.filter(
      (note) => compareVersions(note.version, promptedVersion) > 0,
    ),
  };
}
