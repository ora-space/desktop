import { useEffect } from "react";
import { toast } from "@ora/ui";
import { useTranslation } from "react-i18next";
import { usePlatform } from "../../platform";

/**
 * Turns surface download completions and failures into toasts.
 *
 * Completed downloads offer to reveal the containing folder rather than the
 * archive itself so the host file manager never launches the file.
 */
export function SurfaceDownloadToaster() {
  const { t } = useTranslation();
  const { surfaces } = usePlatform();
  useEffect(() => {
    let disposed = false;
    let unsubscribe: (() => void) | undefined;
    void surfaces
      .onEvent((event) => {
        if (disposed) return;
        if (event.type === "downloadCompleted") {
          // A completion carrying an import session opens the skill-import review
          // (SurfaceDownloadPrompt) instead of a bare toast.
          if (event.importSessionId === null) {
            toast.success(
              t("surface.downloaded", { fileName: event.fileName }),
            );
          }
        } else if (event.type === "downloadFailed") {
          toast.error(
            t("surface.downloadFailed", { fileName: event.fileName }),
            { description: event.reason },
          );
        }
      })
      .then((stop) => {
        if (disposed) stop();
        else unsubscribe = stop;
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
      unsubscribe?.();
    };
  }, [surfaces, t]);
  return null;
}
