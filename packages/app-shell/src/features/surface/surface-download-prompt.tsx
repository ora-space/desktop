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
 */
export function SurfaceDownloadPrompt() {
  const { t } = useTranslation();
  const platform = usePlatform();
  const client = useContractsClient();
  const prompts = useSurfaceStore((state) => state.downloadPrompts);
  const removePrompt = useSurfaceStore((state) => state.removeDownloadPrompt);
  const [busy, setBusy] = useState(false);
  const [importSession, setImportSession] = useState<SkillImportSession | null>(
    null,
  );
  const current = prompts[0] ?? null;

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
            if (!disposed) setImportSession(response.session);
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
        const response = await client.skillImport.get({
          sessionId: outcome.importSessionId,
        });
        setImportSession(response.session);
      }
    } catch (cause) {
      // The host settles a failed action, so the prompt cannot be retried.
      removePrompt(current.downloadId);
      toast.error(t("surface.downloadActionFailed"), {
        description: String(cause),
      });
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
    } catch (cause) {
      removePrompt(current.downloadId);
      toast.error(t("surface.downloadActionFailed"), {
        description: String(cause),
      });
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
            if (!open) setImportSession(null);
          }}
          onCompleted={() => undefined}
          initialSession={importSession}
        />
      )}
    </>
  );
}
