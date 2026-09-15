import { toast } from "@ora/ui";
import { useCallback, useState } from "react";
import { useTranslation } from "react-i18next";
import { useOptionalPlatform } from "../../platform";

/** Drives one plugin log export at a time and reports its outcome through shared toasts. */
export interface PluginLogDownloadController {
  /** True while the host save flow or copy is still running. */
  readonly isDownloading: boolean;
  /** Starts the export; a dismissed save dialog completes silently. */
  readonly download: () => Promise<void>;
}

/**
 * Exports one installed plugin's host-owned log through the platform's save flow. Returns
 * `undefined` when the host cannot export logs (for example the web shell) so callers hide
 * the affordance instead of showing a dead item.
 */
export function usePluginLogDownload(
  pluginId: string,
  displayName: string,
): PluginLogDownloadController | undefined {
  const { t } = useTranslation();
  const diagnosticLogs = useOptionalPlatform()?.diagnosticLogs;
  const [isDownloading, setIsDownloading] = useState(false);

  const download = useCallback(async () => {
    if (diagnosticLogs === undefined) return;
    setIsDownloading(true);
    try {
      const downloaded = await diagnosticLogs.downloadPluginLog(
        pluginId,
        displayName,
      );
      if (downloaded) toast.success(t("settings.plugins.logDownloaded"));
    } catch {
      toast.error(t("settings.plugins.logDownloadFailed"));
    } finally {
      setIsDownloading(false);
    }
  }, [diagnosticLogs, displayName, pluginId, t]);

  return diagnosticLogs === undefined ? undefined : { isDownloading, download };
}
