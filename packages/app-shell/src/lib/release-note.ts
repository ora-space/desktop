import type { Locale } from "../i18n/resource-bundle";

/** In reading order: what needs attention first, then what is new. */
export const RELEASE_NOTE_CATEGORIES = [
  "important",
  "feature",
  "improvement",
  "fix",
] as const;

export type ReleaseNoteCategory = (typeof RELEASE_NOTE_CATEGORIES)[number];

export type ReleaseNoteEntry = {
  category: ReleaseNoteCategory;
} & Record<Locale, string>;

/** One stable version's notes, shared by the bundled notes and the update manifest. */
export interface ReleaseNote {
  version: string;
  date: string;
  entries: ReleaseNoteEntry[];
}
