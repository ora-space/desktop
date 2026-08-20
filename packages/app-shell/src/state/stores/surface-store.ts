import { create } from "zustand";
import type { SurfaceEvent, SurfaceRecord } from "../../platform";

interface SurfaceStoreState {
  /** Whether the host can embed surfaces as child webviews; null until queried. */
  embeddedSupported: boolean | null;
  /** Live surface instances keyed by their host-assigned instance id. */
  records: Record<number, SurfaceRecord>;
  /** Last failure reason per instance, shown by the host's error state. */
  failures: Record<number, string>;
  /** The embedded instance currently occupying the right review slot. */
  sidePanelInstance: number | null;
  setEmbeddedSupported(embedded: boolean): void;
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
  setEmbeddedSupported: (embedded) => set({ embeddedSupported: embedded }),
  hydrate: (records) =>
    set({
      records: Object.fromEntries(
        records.map((record) => [record.instance, record]),
      ),
    }),
  setSidePanelInstance: (instance) => set({ sidePanelInstance: instance }),
  applyEvent: (event) =>
    set((state) => {
      switch (event.type) {
        case "opened":
          return {
            records: {
              ...state.records,
              [event.instance]: {
                instance: event.instance,
                pluginId: event.pluginId,
                surfaceId: event.surfaceId,
                title: event.title,
                target: event.target,
                state: "open",
              },
            },
          };
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
        case "downloadStarted":
        case "downloadCompleted":
        case "downloadFailed":
          return state;
      }
    }),
}));
