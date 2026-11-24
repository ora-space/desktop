import { create } from "zustand";
import type { ReleaseNotesPrompt } from "./release-notes-model";

interface ReleaseNotesState {
  /** The open prompt, or null before one is planned and after it is dismissed. */
  prompt: ReleaseNotesPrompt | null;
  open(prompt: ReleaseNotesPrompt): void;
  dismiss(): void;
}

/**
 * Keeps the open prompt for the lifetime of the process. The shell unmounts while the application
 * event stream reconnects, and by then the version is already remembered as prompted, so state held
 * by the card would lose notes the user had not dismissed.
 */
export const useReleaseNotesStore = create<ReleaseNotesState>()((set) => ({
  prompt: null,
  open: (prompt) => set({ prompt }),
  dismiss: () => set({ prompt: null }),
}));
