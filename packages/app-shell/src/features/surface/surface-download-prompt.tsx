import { useEffect, useState } from "react";
import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  toast,
} from "@ora/ui";
import { useTranslation } from "react-i18next";
import type { SkillImportSession } from "@ora/contracts";
import { usePlatform } from "../../platform";
import { useContractsClient } from "../../contracts-client-context";
import { useSurfaceStore } from "../../state/stores/surface-store";
import { SkillImportDialog } from "../settings/atoms-settings";
import { useOpenSurface } from "./use-open-surface";

/** Renders a byte count as a short human-readable size for the prompt copy. */
function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/**
 * The user-facing side of prompt-disposition webview downloads.
 *
 * The host parks a landed prompt download in `AwaitingChoice` until the trusted
 * main webview answers; this dialog shows the queued prompts one at a time and
 * answers with `resolveDownload` (running the chosen host action) or
 * `discardDownload` (dismiss, which also deletes the landed file). A resolved
 * `import_skill` — and likewise an automatic import completed by the host —
 * opens the shared skill-import review dialog on the prepared session.
 *
 * Answering the last queued prompt also re-opens the surface the download came
 * from: the host raised the main window to show this dialog, so returning the
 * plugin view to the front is what restores the user's browsing context.
 */
export function SurfaceDownloadPrompt() {
  const { t } = useTranslation();
  const platform = usePlatform();
  const client = useContractsClient();
  const openSurface = useOpenSurface();
  const prompts = useSurfaceStore((state) => state.downloadPrompts);
  const removePrompt = useSurfaceStore((state) => state.removeDownloadPrompt);
  const [busy, setBusy] = useState(false);
  const [importSession, setImportSession] = useState<SkillImportSession | null>(
    null,
  );
  /**
   * The plugin whose surface re-opens once the import review dialog closes; the
   * marketplace must not come back while that dialog still needs the screen.
   */
  const [restoreSurfaceId, setRestoreSurfaceId] = useState<string | null>(null);
  const current = prompts[0] ?? null;
  /** Answering the last prompt ends the chain; earlier ones keep the dialog up. */
  const lastPrompt = prompts.length <= 1;
  const restoreSurface = (pluginId: string) => {
    if (lastPrompt) void openSurface({ pluginId });
  };

  // Automatic `import_skill` dispositions complete host-side; their event carries
  // the prepared session so the review opens without any prompt round trip.
  useEffect(() => {
    let disposed = false;
    let unsubscribe: (() => void) | undefined;
    void platform.surfaces
      .onEvent((event) => {
        if (
          disposed ||
          event.type !== "downloadCompleted" ||
          event.importSessionId === null
        ) {
          return;
        }
        void client.skillImport
          .get({ sessionId: event.importSessionId })
          .then((response) => {
            if (!disposed) {
              setRestoreSurfaceId(event.pluginId);
              setImportSession(response.session);
            }
          })
          .catch(() => {
            toast.error(t("surface.downloadActionFailed"));
          });
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
  }, [client, platform, t]);

  const dismiss = () => {
    if (current === null) return;
    void platform.surfaces
      .discardDownload(current.downloadId)
      .catch(() => undefined);
    restoreSurface(current.pluginId);
    removePrompt(current.downloadId);
  };

  const importSkill = async () => {
    if (current === null) return;
    setBusy(true);
    try {
      const outcome = await platform.surfaces.resolveDownload(
        current.downloadId,
        "import_skill",
      );
      removePrompt(current.downloadId);
      if (outcome.importSessionId !== null) {
        // The review dialog is the next modal step; the marketplace comes back
        // when it closes (see the dialog's onOpenChange).
        if (lastPrompt) setRestoreSurfaceId(current.pluginId);
        const response = await client.skillImport.get({
          sessionId: outcome.importSessionId,
        });
        setImportSession(response.session);
      } else {
        restoreSurface(current.pluginId);
      }
    } catch (cause) {
      // The host settles a failed action, so the prompt cannot be retried.
      removePrompt(current.downloadId);
      toast.error(t("surface.downloadActionFailed"), {
        description: String(cause),
      });
      restoreSurface(current.pluginId);
    } finally {
      setBusy(false);
    }
  };

  const saveAs = async () => {
    if (current === null) return;
    const destination = await platform.selectSavePath({
      defaultFileName: current.fileName,
    });
    // A dismissed save dialog keeps the prompt open so the user can pick again.
    if (destination === null) return;
    setBusy(true);
    try {
      await platform.surfaces.resolveDownload(
        current.downloadId,
        "save_as",
        destination,
      );
      removePrompt(current.downloadId);
      toast.success(t("surface.downloaded", { fileName: current.fileName }));
      restoreSurface(current.pluginId);
    } catch (cause) {
      removePrompt(current.downloadId);
      toast.error(t("surface.downloadActionFailed"), {
        description: String(cause),
      });
      restoreSurface(current.pluginId);
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <Dialog
        open={current !== null}
        onOpenChange={(open) => {
          if (!open) dismiss();
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("surface.downloadChoiceTitle")}</DialogTitle>
            <DialogDescription>
              {current !== null &&
                t("surface.downloadChoiceDescription", {
                  origin: current.pageOrigin,
                  fileName: current.fileName,
                  size: formatSize(current.sizeBytes),
                })}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="ghost" disabled={busy} onClick={dismiss}>
              {t("surface.downloadDismiss")}
            </Button>
            {current?.actions.includes("save_as") && (
              <Button
                variant="secondary"
                disabled={busy}
                onClick={() => void saveAs()}
              >
                {t("surface.downloadSaveAs")}
              </Button>
            )}
            {current?.actions.includes("import_skill") && (
              <Button disabled={busy} onClick={() => void importSkill()}>
                {t("surface.downloadImportSkill")}
              </Button>
            )}
          </DialogFooter>
        </DialogContent>
      </Dialog>
      {importSession !== null && (
        // Keyed by session so a later import mounts a fresh dialog; the shared
        // dialog freezes its initial session on first render.
        <SkillImportDialog
          key={importSession.sessionId}
          open
          onOpenChange={(open) => {
            if (!open) {
              setImportSession(null);
              // The whole chain ended with the review: hand the screen back to
              // the marketplace the download came from.
              const pluginId = restoreSurfaceId;
              setRestoreSurfaceId(null);
              if (pluginId !== null) void openSurface({ pluginId });
            }
          }}
          onCompleted={() => undefined}
          initialSession={importSession}
          onSessionEnded={
            restoreSurfaceId !== null
              ? () => {
                  // Re-choosing or continuing the next import from a
                  // marketplace download means going back to the marketplace,
                  // so the review dialog closes and the surface comes back to
                  // the front.
                  const pluginId = restoreSurfaceId;
                  setImportSession(null);
                  setRestoreSurfaceId(null);
                  void openSurface({ pluginId });
                }
              : undefined
          }
        />
      )}
    </>
  );
}
