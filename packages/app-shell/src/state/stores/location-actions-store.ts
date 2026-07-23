import { create } from "zustand";
import { persist, createJSONStorage } from "zustand/middleware";
import type { LocationTarget } from "@ora/platform";

/**
 * The opener the split button repeats when its main (icon) half is clicked. Every
 * `LocationTarget` is eligible; "Copy Path" is a menu-only action and never a default,
 * so it is deliberately not part of this type.
 */
export type DefaultLocationTarget = LocationTarget;

const DEFAULT_TARGET: DefaultLocationTarget = "explorer";
const STORAGE_KEY = "ora.location-actions.v1";

const VALID_TARGETS: readonly DefaultLocationTarget[] = ["explorer", "terminal", "vscode"] as const;

interface LocationActionsState {
  /** The opener the folder/main button executes, remembered across sessions. */
  defaultTarget: DefaultLocationTarget;
  /** Records the opener the user just chose so the main button repeats it next time. */
  setDefaultTarget: (target: DefaultLocationTarget) => void;
}

/**
 * Remembers the split button's default opener in localStorage only - the backend never
 * learns which editor a given machine prefers, matching how this preference is purely a
 * client convenience.
 */
export const useLocationActionsStore = create<LocationActionsState>()(
  persist(
    (set) => ({
      defaultTarget: DEFAULT_TARGET,
      setDefaultTarget: (target) => set({ defaultTarget: target }),
    }),
    {
      name: STORAGE_KEY,
      storage: createJSONStorage(() => window.localStorage),
      // Fall back to Explorer if the persisted value is missing or no longer a known target.
      merge: (persisted, current) => {
        const persistedTarget = (persisted as Partial<LocationActionsState> | undefined)
          ?.defaultTarget;
        return {
          ...current,
          defaultTarget: VALID_TARGETS.includes(persistedTarget as DefaultLocationTarget)
            ? (persistedTarget as DefaultLocationTarget)
            : DEFAULT_TARGET,
        };
      },
    },
  ),
);
