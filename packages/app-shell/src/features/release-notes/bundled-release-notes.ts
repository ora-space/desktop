import type { Locale } from "../../i18n/resource-bundle";

const CATEGORIES = ["important", "feature", "improvement", "fix"] as const;

export type ReleaseNoteCategory = (typeof CATEGORIES)[number];

export type ReleaseNoteEntry = {
  category: ReleaseNoteCategory;
} & Record<Locale, string>;

/** One stable version's notes, as validated and injected when the Desktop frontend was built. */
export interface ReleaseNote {
  version: string;
  date: string;
  entries: ReleaseNoteEntry[];
}

/**
 * The Desktop build injects every notes file of its checkout. Other hosts and tests leave it unset,
 * which reads as a build without notes.
 */
declare const __ORA_RELEASE_NOTES__: unknown;

/** Recognizes the injected shape, so a malformed injection hides notes instead of breaking the app. */
function isReleaseNote(value: unknown): value is ReleaseNote {
  if (typeof value !== "object" || value === null) return false;
  const { version, date, entries } = value as Partial<ReleaseNote>;
  return (
    typeof version === "string" &&
    typeof date === "string" &&
    Array.isArray(entries) &&
    entries.every(
      (entry) =>
        typeof entry === "object" &&
        entry !== null &&
        CATEGORIES.includes(entry.category) &&
        typeof entry["zh-CN"] === "string" &&
        typeof entry["en-US"] === "string",
    )
  );
}

/** Accepts the injected notes, or none when any of them is malformed. */
export function decodeReleaseNotes(value: unknown): readonly ReleaseNote[] {
  return Array.isArray(value) && value.every(isReleaseNote) ? value : [];
}

export const bundledReleaseNotes = decodeReleaseNotes(
  typeof __ORA_RELEASE_NOTES__ === "undefined" ? [] : __ORA_RELEASE_NOTES__,
);
