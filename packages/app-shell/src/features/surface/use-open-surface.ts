import { useCallback } from "react";
import { toast } from "@ora/ui";
import { useTranslation } from "react-i18next";
import { usePlatform, type SurfaceOpenTarget } from "../../platform";
import { useSurfaceStore } from "../../state/stores/surface-store";

/**
 * Opens a plugin surface embedded when the host supports it, otherwise windowed.
 *
 * An embedded result claims the right review slot; the review layout reacts to
 * the store and slides its panel open. A windowed result needs no shell action
 * because the host already focused the window. Failures surface as a toast.
 */
export function useOpenSurface(): (
  definition: SurfaceOpenTarget,
) => Promise<void> {
  const { t } = useTranslation();
  const { surfaces } = usePlatform();
  return useCallback(
    async (definition) => {
      const { embeddedSupported, setSidePanelInstance, applyEvent } =
        useSurfaceStore.getState();
      const target = embeddedSupported ? "embedded" : "windowed";
      try {
        const record = await surfaces.open(
          { pluginId: definition.pluginId },
          target,
        );
        // The `opened` event may race the response; seeding the record keeps the
        // host header populated either way.
        applyEvent({ type: "opened", ...record });
        if (record.target === "embedded") {
          setSidePanelInstance(record.instance);
        }
      } catch {
        toast.error(t("surface.openFailed"));
      }
    },
    [surfaces, t],
  );
}
