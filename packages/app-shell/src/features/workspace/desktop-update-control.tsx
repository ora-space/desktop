import { useTranslation } from "react-i18next";
import {
  IconAlertTriangle,
  IconDownload,
  IconLoader2,
  IconRefresh,
} from "@tabler/icons-react";
import { Button, cn } from "@ora/ui";
import { appVersion } from "../../lib/app-version";
import type { DesktopUpdateStatus } from "../../platform/types";
import { useDesktopUpdate } from "./use-desktop-update";

/** Formats a byte count the way the download progress reads in both locales. */
function formatMegabytes(bytes: number): string {
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/** Renders the plain version stamp used by hosts that cannot update themselves. */
function VersionStamp() {
  const { t } = useTranslation();
  return (
    <p className="px-2 pb-1 text-xs leading-5 text-muted-foreground">
      {t("update.currentVersion", { version: appVersion })}
    </p>
  );
}

/** Renders the version stamp and leads to the offered release, or to why the last check failed. */
export function DesktopUpdateControl({ onReview }: { onReview: () => void }) {
  const { t } = useTranslation();
  const { updates, status } = useDesktopUpdate();

  // The web host has no updater, but the settings sidebar still stamps the running version.
  if (updates === undefined) return <VersionStamp />;

  // A manual update is reviewed like an installable one; only its install action is withheld.
  const reviewable = status.kind === "ready" || status.kind === "manual_update";
  const failed = status.kind === "failed";
  const busy =
    status.kind === "checking" ||
    status.kind === "downloading" ||
    status.kind === "installing";

  return (
    <div>
      {/* A tooltip alone would leave a check the user asked for looking like it did nothing. */}
      {failed && (
        <p
          role="status"
          className="px-2 pb-1 text-xs leading-5 text-destructive"
        >
          {status.operation === "install"
            ? t("update.installFailed", { version: status.version })
            : t("update.failedTitle")}
        </p>
      )}
      <div className="flex items-center gap-1">
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="relative h-7 flex-1 justify-start px-2 text-xs text-muted-foreground"
          disabled={!reviewable && !failed}
          title={describeStatus(status, t)}
          aria-label={describeStatus(status, t)}
          onClick={onReview}
        >
          {busy ? (
            <IconLoader2 className="mr-1.5 size-3.5 animate-spin" />
          ) : failed ? (
            <IconAlertTriangle className="mr-1.5 size-3.5 text-destructive" />
          ) : (
            <IconDownload className="mr-1.5 size-3.5" />
          )}
          <span className="truncate">
            {t("update.currentVersion", { version: appVersion })}
          </span>
          {reviewable && (
            <span
              data-testid="desktop-update-badge"
              className="absolute right-2 top-1/2 size-2 -translate-y-1/2 rounded-full bg-destructive"
            />
          )}
        </Button>
        <Button
          type="button"
          variant="ghost"
          size="icon"
          className="size-7 shrink-0 text-muted-foreground"
          disabled={busy}
          title={t("update.checkNow")}
          aria-label={t("update.checkNow")}
          onClick={() => void updates.check()}
        >
          <IconRefresh
            className={cn(
              "size-3.5",
              status.kind === "checking" && "animate-spin",
            )}
          />
        </Button>
      </div>
    </div>
  );
}

/** Renders the status as the one line shown in the tooltip and to assistive technology. */
function describeStatus(
  status: DesktopUpdateStatus,
  t: (key: string, options?: Record<string, unknown>) => string,
): string {
  switch (status.kind) {
    case "current":
      return t("update.currentVersion", { version: appVersion });
    case "checking":
      return t("update.checking");
    case "downloading":
      return status.total === null
        ? t("update.downloadingUnknownSize", { version: status.version })
        : t("update.downloading", {
            version: status.version,
            progress: `${formatMegabytes(status.downloaded)} / ${formatMegabytes(status.total)}`,
          });
    case "ready":
      return t("update.ready", { version: status.version });
    case "manual_update":
      return t(`update.manual.${status.reason}`, { version: status.version });
    case "installing":
      return t("update.installing", { version: status.version });
    case "failed":
      return status.operation === "install"
        ? t("update.installFailedDetail", {
            version: status.version,
            message: status.message,
          })
        : t("update.failed", { message: status.message });
  }
}
