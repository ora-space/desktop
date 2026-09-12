import { toast } from "@ora/ui";
import { useCallback } from "react";
import { useTranslation } from "react-i18next";
import { useDiagnosticLogDownload } from "../state/hooks/use-diagnostic-log-download";
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
  const logDownload = useDiagnosticLogDownload();

  return useCallback(
    (error: unknown, title?: string) => {
      const message = localizeContractError(error, t);
      const action =
        logDownload !== undefined && hasDiagnosticRequestId(error)
          ? {
              label: t("errors.downloadLogs"),
              onClick: () => void logDownload.download(),
            }
          : undefined;

      toast.error(title ?? message, {
        description: title === undefined ? undefined : message,
        action,
      });
    },
    [logDownload, t],
  );
}
