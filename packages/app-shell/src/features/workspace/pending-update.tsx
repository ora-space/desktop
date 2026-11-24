import { useState } from "react";
import { useTranslation } from "react-i18next";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  Button,
} from "@ora/ui";
import { appVersion } from "../../lib/app-version";
import { ReleaseNoteList } from "../release-notes/release-note-list";
import { useDesktopUpdate } from "./use-desktop-update";

/**
 * Presents the release an update would bring, with its notes, before the user installs it. Nothing is
 * rendered unless a release is ready or must be installed by other means.
 */
export function PendingUpdate() {
  const { t } = useTranslation();
  const { updates, status } = useDesktopUpdate();
  const [confirming, setConfirming] = useState(false);
  const installable = status.kind === "ready" ? status.version : null;
  const [lastInstallable, setLastInstallable] = useState(installable);
  // A confirmation is for the release the user was reading; a background check can replace it.
  if (lastInstallable !== installable) {
    setLastInstallable(installable);
    setConfirming(false);
  }
  if (
    updates === undefined ||
    (status.kind !== "ready" && status.kind !== "manual_update")
  ) {
    return null;
  }
  const { version, notes } = status;
  const title = t("update.available", { version });
  return (
    <section
      aria-label={title}
      className="rounded-xl border border-border bg-muted/30 p-5"
    >
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <h3 className="text-sm font-semibold">{title}</h3>
          <p className="mt-1 text-xs leading-5 text-muted-foreground">
            {status.kind === "ready"
              ? t("update.availableDescription", {
                  currentVersion: appVersion,
                  version,
                })
              : t(`update.manual.${status.reason}`, { version })}
          </p>
        </div>
        {status.kind === "ready" && (
          <Button
            type="button"
            size="sm"
            className="shrink-0"
            onClick={() => setConfirming(true)}
          >
            {t("update.confirmAction")}
          </Button>
        )}
      </div>
      {notes.length > 0 && (
        <div className="mt-4 border-t border-border/70 pt-4">
          <ReleaseNoteList notes={notes} />
        </div>
      )}
      {/* Installing restarts the app without checking for running work, so it is confirmed once. */}
      <AlertDialog open={confirming} onOpenChange={setConfirming}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t("update.confirmTitle", { version })}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t("update.confirmDescription", {
                currentVersion: appVersion,
                version,
              })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                setConfirming(false);
                void updates.install(version);
              }}
            >
              {t("update.confirmAction")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </section>
  );
}
