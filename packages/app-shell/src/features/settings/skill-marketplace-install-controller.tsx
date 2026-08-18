import { useCallback, useEffect, useRef, useState } from "react";
import { type SkillImportSession } from "@ora/contracts";
import { usePlatform, type SkillMarketplaceStatus } from "../../platform";
import { toast } from "@ora/ui";
import { useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { useContractsClient } from "../../contracts-client-context";
import { localizeContractError } from "../../i18n/contract-error";
import {
  localizeSkillImportResultReason,
  localizeSkillImportStatus,
} from "../../i18n/skill-import-reason";
import { queryKeys } from "../../state/hooks/query-keys";
import { SkillImportDialog } from "./atoms-settings";

type DownloadedMarketplaceArchive = Extract<
  SkillMarketplaceStatus,
  { status: "downloaded" }
>;

interface PendingMarketplaceReview {
  archive: DownloadedMarketplaceArchive;
  session: SkillImportSession;
  resolve: () => void;
}

/** Installs marketplace archives through the existing two-phase skill import workflow. */
export function SkillMarketplaceInstallController() {
  const { t } = useTranslation();
  const client = useContractsClient();
  const queryClient = useQueryClient();
  const { skillMarketplace } = usePlatform();
  const installQueue = useRef<Promise<void>>(Promise.resolve());
  const handledArchives = useRef(new Set<string>());
  const [review, setReview] = useState<PendingMarketplaceReview | null>(null);

  /** Refreshes installed skills after either automatic or reviewed imports finish. */
  const refreshSkills = useCallback(
    () => queryClient.invalidateQueries({ queryKey: queryKeys.skills }),
    [queryClient],
  );

  /** Pauses the FIFO queue while the shared import dialog owns a non-ready session. */
  const requestReview = useCallback(
    (archive: DownloadedMarketplaceArchive, session: SkillImportSession) =>
      new Promise<void>((resolve) => {
        setReview({ archive, session, resolve });
      }),
    [],
  );

  /** Prepares, commits, and observes one downloaded archive without bypassing import validation. */
  const installArchive = useCallback(
    async (archive: DownloadedMarketplaceArchive) => {
      toast(
        t("settings.skills.marketplaceInstalling", {
          fileName: archive.fileName,
        }),
        {
          description: archive.archivePath,
        },
      );
      try {
        const prepared = await client.skillImport.prepare({
          source: {
            kind: "archive",
            path: archive.archivePath,
            fileName: archive.fileName,
          },
        });
        const needsReview = prepared.session.candidates.some(
          (candidate) => candidate.status !== "ready",
        );
        if (needsReview) {
          toast(
            t("settings.skills.marketplaceInstallReview", {
              fileName: archive.fileName,
            }),
          );
          await requestReview(archive, prepared.session);
          return;
        }

        await client.skillImport.commit({
          sessionId: prepared.session.sessionId,
          decisions: [],
        });

        let completed = prepared.session;
        do {
          await new Promise<void>((resolve) => window.setTimeout(resolve, 750));
          completed = (
            await client.skillImport.get({
              sessionId: prepared.session.sessionId,
            })
          ).session;
        } while (completed.status !== "completed");

        await refreshSkills();
        const incomplete = completed.progress.results.some(
          (result) =>
            result.status !== "imported" && result.status !== "overwritten",
        );
        if (incomplete) {
          const reasons = completed.progress.results
            .filter(
              (result) =>
                result.status !== "imported" && result.status !== "overwritten",
            )
            .map((result) => {
              const reason =
                localizeSkillImportResultReason(result, t) ??
                localizeSkillImportStatus(result.status, t);
              return t("settings.skills.importResultLine", {
                name: result.name,
                reason,
              });
            })
            .join("\n");
          toast.error(
            t("settings.skills.marketplaceInstallIncomplete", {
              fileName: archive.fileName,
            }),
            { description: reasons },
          );
          await requestReview(archive, completed);
          return;
        }

        toast.success(
          t("settings.skills.marketplaceInstalled", {
            count: completed.progress.results.length,
          }),
        );
      } catch (cause) {
        toast.error(
          t("settings.skills.marketplaceInstallFailed", {
            fileName: archive.fileName,
          }),
          {
            description: localizeContractError(cause, t),
          },
        );
      }
    },
    [client, refreshSkills, requestReview, t],
  );

  useEffect(() => {
    let disposed = false;
    let unsubscribe: (() => void) | undefined;
    void skillMarketplace
      .onStatus((status) => {
        if (disposed || status.status !== "downloaded") return;
        if (handledArchives.current.has(status.archivePath)) return;
        handledArchives.current.add(status.archivePath);
        // Serializing installation avoids presenting two conflict dialogs or racing same-name imports.
        installQueue.current = installQueue.current.then(() =>
          installArchive(status),
        );
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
  }, [installArchive, skillMarketplace]);

  /** Releases the queued archive after the shared dialog is closed or completes. */
  const finishReview = () => {
    review?.resolve();
    setReview(null);
  };

  if (review === null) return null;

  return (
    <SkillImportDialog
      open
      initialSession={review.session}
      onOpenChange={(open) => {
        if (!open) finishReview();
      }}
      onCompleted={() => {
        void refreshSkills();
        toast.success(t("settings.skills.marketplaceInstallReviewed"));
        finishReview();
      }}
    />
  );
}
