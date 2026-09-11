import { Button } from "@ora/ui";
import { IconDownload } from "@tabler/icons-react";
import { useTranslation } from "react-i18next";
import { useDiagnosticLogDownload } from "../../state/hooks/use-diagnostic-log-download";

/**
 * Lets developers export today's diagnostic log without waiting for an error toast.
 * Renders nothing when the host cannot export logs so the web shell shows no dead button.
 */
export function DiagnosticLogsSettings() {
  const { t } = useTranslation();
  const logDownload = useDiagnosticLogDownload();
  if (logDownload === undefined) return null;

  return (
    <section
      className="border-b border-border py-4"
      aria-labelledby="diagnostic-logs-title"
    >
      <div className="flex flex-col gap-3 sm:flex-row sm:items-center">
        <IconDownload className="hidden size-4 shrink-0 text-muted-foreground sm:block" />
        <div className="min-w-0 flex-1">
          <p id="diagnostic-logs-title" className="text-sm font-medium">
            {t("settings.developer.diagnosticLogs")}
          </p>
          <p className="mt-1 text-xs leading-5 text-muted-foreground">
            {t("settings.developer.diagnosticLogsDescription")}
          </p>
        </div>
        <Button
          size="sm"
          variant="outline"
          className="shrink-0"
          disabled={logDownload.isDownloading}
          onClick={() => void logDownload.download()}
        >
          {logDownload.isDownloading
            ? t("settings.developer.downloadLogsInProgress")
            : t("settings.developer.downloadLogs")}
        </Button>
      </div>
    </section>
  );
}
