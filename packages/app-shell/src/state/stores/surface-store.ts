import { create } from "zustand";
import type {
  DownloadAction,
  SurfaceEvent,
  SurfaceRecord,
} from "../../platform";

/** One landed prompt-disposition download waiting for the user to pick a host action. */
export interface SurfaceDownloadPromptEntry {
  instance: number;
  pluginId: string;
  downloadId: number;
  pageOrigin: string;
  fileName: string;
  sizeBytes: number;
  actions: DownloadAction[];
}

interface SurfaceStoreState {
  /** Whether the host can embed surfaces as child webviews; null until queried. */
  embeddedSupported: boolean | null;
  /** Live surface instances keyed by their host-assigned instance id. */
  records: Record<number, SurfaceRecord>;
  /** Last failure reason per instance, shown by the host's error state. */
  failures: Record<number, string>;
  /** The embedded instance currently occupying the right review slot. */
  sidePanelInstance: number | null;
  /**
   * Bumped on every claim so re-claiming the instance already in the slot stays
   * observable; a value-only comparison cannot distinguish it from no-op event
   * traffic, which must not yank the review panel back to the surface.
   */
  sidePanelClaimTick: number;
  /** Landed prompt downloads in arrival order; the prompt dialog shows the head. */
  downloadPrompts: SurfaceDownloadPromptEntry[];
  setEmbeddedSupported(embedded: boolean): void;
  /** Drops one prompt after the user resolved or dismissed it. */
  removeDownloadPrompt(downloadId: number): void;
  /** Replaces the record set with the host's snapshot (startup / reconnect). */
  hydrate(records: SurfaceRecord[]): void;
  /** Claims the right slot for an embedded instance, or releases it with null. */
  setSidePanelInstance(instance: number | null): void;
  /** Folds one lifecycle event into the record set; download events are ignored here. */
  applyEvent(event: SurfaceEvent): void;
}

/** Removes one instance and releases the side slot if that instance owned it. */
function withoutInstance(
  state: SurfaceStoreState,
  instance: number,
): Partial<SurfaceStoreState> {
  const records = { ...state.records };
  delete records[instance];
  const failures = { ...state.failures };
  delete failures[instance];
  return {
    records,
    failures,
    sidePanelInstance:
      state.sidePanelInstance === instance ? null : state.sidePanelInstance,
  };
}

/** Mirrors the host's surface registry so the launcher, panel, and toaster share one view. */
export const useSurfaceStore = create<SurfaceStoreState>()((set) => ({
  embeddedSupported: null,
  records: {},
  failures: {},
  sidePanelInstance: null,
  sidePanelClaimTick: 0,
  downloadPrompts: [],
  setEmbeddedSupported: (embedded) => set({ embeddedSupported: embedded }),
  removeDownloadPrompt: (downloadId) =>
    set((state) => ({
      downloadPrompts: state.downloadPrompts.filter(
        (prompt) => prompt.downloadId !== downloadId,
      ),
    })),
  hydrate: (records) =>
    set({
      records: Object.fromEntries(
        records.map((record) => [record.instance, record]),
      ),
    }),
  setSidePanelInstance: (instance) =>
    set((state) => ({
      sidePanelInstance: instance,
      // Releases keep the tick: they must collapse the panel, not re-open it.
      sidePanelClaimTick:
        instance === null
          ? state.sidePanelClaimTick
          : state.sidePanelClaimTick + 1,
    })),
  applyEvent: (event) =>
    set((state) => {
      switch (event.type) {
        case "opened": {
          // A rebuilt instance reuses its id, so a successful retry must also drop the
          // failure reason the host's error state was showing.
          const failures = { ...state.failures };
          delete failures[event.instance];
          return {
            failures,
            records: {
              ...state.records,
              [event.instance]: {
                instance: event.instance,
                pluginId: event.pluginId,
                kind: event.kind,
                title: event.title,
                target: event.target,
                state: "open",
              },
            },
          };
        }
        case "migrated": {
          const record = state.records[event.instance];
          // A surface docked back from a window takes the side slot; one popped
          // out releases it so the review panel can close.
          const sidePanelInstance =
            event.target === "embedded"
              ? event.instance
              : state.sidePanelInstance === event.instance
                ? null
                : state.sidePanelInstance;
          return {
            sidePanelInstance,
            records:
              record === undefined
                ? state.records
                : {
                    ...state.records,
                    [event.instance]: {
                      ...record,
                      target: event.target,
                      state: "open",
                    },
                  },
          };
        }
        case "migrateFailed": {
          const record = state.records[event.instance];
          if (record === undefined) return state;
          // The host keeps the surface where it was, so only the transient state resets.
          return {
            records: {
              ...state.records,
              [event.instance]: { ...record, state: "open" },
            },
          };
        }
        case "failed": {
          const record = state.records[event.instance];
          return {
            failures: { ...state.failures, [event.instance]: event.reason },
            records:
              record === undefined
                ? state.records
                : {
                    ...state.records,
                    [event.instance]: { ...record, state: "failed" },
                  },
          };
        }
        case "closed":
          return withoutInstance(state, event.instance);
        case "downloadChoice": {
          // Host retries cannot re-announce the same download; keep the queue id-unique anyway
          // so a duplicated event never shows two dialogs for one file.
          if (
            state.downloadPrompts.some(
              (prompt) => prompt.downloadId === event.downloadId,
            )
          ) {
            return state;
          }
          return {
            downloadPrompts: [
              ...state.downloadPrompts,
              {
                instance: event.instance,
                pluginId: event.pluginId,
                downloadId: event.downloadId,
                pageOrigin: event.pageOrigin,
                fileName: event.fileName,
                sizeBytes: event.sizeBytes,
                actions: event.actions,
              },
            ],
          };
        }
        case "downloadStarted":
        case "downloadCompleted":
        case "downloadFailed":
          return state;
      }
    }),
}));
