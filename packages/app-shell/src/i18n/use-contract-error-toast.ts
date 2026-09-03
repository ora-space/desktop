import { toast } from "@ora/ui";
import { useCallback } from "react";
import { useTranslation } from "react-i18next";
import { useOptionalPlatform } from "../platform";
import {
  hasDiagnosticRequestId,
  localizeContractError,
} from "./contract-error";

/** Shows contract failures consistently and offers logs when the message asks for a request ID. */
export function useContractErrorToast(): (
  error: unknown,
  title?: string,
) => void {
  const { t } = useTranslation();
  const diagnosticLogs = useOptionalPlatform()?.diagnosticLogs;

  return useCallback(
    (error: unknown, title?: string) => {
      const message = localizeContractError(error, t);
      const action =
        diagnosticLogs !== undefined && hasDiagnosticRequestId(error)
          ? {
              label: t("errors.downloadLogs"),
              onClick: () => {
                void diagnosticLogs
                  .downloadToday()
                  .then((downloaded) => {
                    if (downloaded) toast.success(t("errors.logsDownloaded"));
                  })
                  .catch(() => toast.error(t("errors.logsDownloadFailed")));
              },
            }
          : undefined;

      toast.error(title ?? message, {
        description: title === undefined ? undefined : message,
        action,
      });
    },
    [diagnosticLogs, t],
  );
}
