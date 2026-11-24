import { stableVersionOf } from "./release-notes-model";

const STORAGE_KEY = "ora.releaseNotes.promptedVersion";

/** Reads the highest prompted version; an unreadable or malformed record counts as none. */
export function readPromptedVersion(): string | null {
  try {
    const stored = window.localStorage.getItem(STORAGE_KEY);
    return stored === null ? null : stableVersionOf(stored);
  } catch {
    return null;
  }
}

/** Remembers the prompted version; a lost write can only repeat the prompt. */
export function writePromptedVersion(version: string): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, version);
  } catch {
    // Best effort: showing notes must not depend on storage availability.
  }
}
