import { toast } from "@ora/ui";
import { useCallback, useState } from "react";
import { useTranslation } from "react-i18next";
import { useOptionalPlatform } from "../../platform";

/** Drives one host log export at a time and reports its outcome through shared toasts. */
export interface DiagnosticLogDownloadController {
  /** True while the host save flow or copy is still running. */
  readonly isDownloading: boolean;
  /** Starts the export; a dismissed save dialog completes silently. */
  readonly download: () => Promise<void>;
}

/**
 * Shares the diagnostic log export between the error toast action and Developer options
 * so both surfaces show identical success and failure messaging. Returns `undefined` when
 * the host cannot export logs (for example the web shell) so callers hide the affordance.
 */
export function useDiagnosticLogDownload():
  DiagnosticLogDownloadController | undefined {
  const { t } = useTranslation();
  const diagnosticLogs = useOptionalPlatform()?.diagnosticLogs;
  const [isDownloading, setIsDownloading] = useState(false);

  const download = useCallback(async () => {
    if (diagnosticLogs === undefined) return;
    setIsDownloading(true);
    try {
      const downloaded = await diagnosticLogs.downloadToday();
      if (downloaded) toast.success(t("errors.logsDownloaded"));
    } catch {
      toast.error(t("errors.logsDownloadFailed"));
    } finally {
      setIsDownloading(false);
    }
  }, [diagnosticLogs, t]);

  return diagnosticLogs === undefined ? undefined : { isDownloading, download };
}
