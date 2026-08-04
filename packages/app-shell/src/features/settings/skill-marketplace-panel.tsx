import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button, toast } from "@ora/ui";
import { IconExternalLink, IconFolderOpen, IconShoppingBag } from "@tabler/icons-react";
import { usePlatform, type SkillMarketplaceStatus } from "@ora/platform";

/** Opens SkillHub and keeps the latest native download status visible in the Skills pane. */
export function SkillMarketplacePanel() {
  const { t } = useTranslation();
  const { locationActions, skillMarketplace } = usePlatform();
  const [status, setStatus] = useState<SkillMarketplaceStatus | null>(null);
  const [opening, setOpening] = useState(false);
  const [connectionFailed, setConnectionFailed] = useState(false);

  useEffect(() => {
    if (skillMarketplace.kind !== "supported") return undefined;

    let disposed = false;
    let unsubscribe: (() => void) | undefined;
    void skillMarketplace
      .onStatus((nextStatus) => {
        if (disposed) return;

        setStatus(nextStatus);
        if (nextStatus.status === "downloaded") {
          toast.success(
            t("settings.skills.marketplaceDownloaded", { fileName: nextStatus.fileName }),
            {
              description: nextStatus.archivePath,
              duration: 5_000,
            },
          );
        }
      })
      .then((stop) => {
        if (disposed) stop();
        else unsubscribe = stop;
      })
      .catch(() => {
        if (!disposed) setConnectionFailed(true);
      });

    return () => {
      disposed = true;
      unsubscribe?.();
    };
  }, [skillMarketplace, t]);

  /** Opens or focuses the native SkillHub window while preventing duplicate button actions. */
  const openMarketplace = async () => {
    if (skillMarketplace.kind !== "supported") return;
    setOpening(true);
    setConnectionFailed(false);
    try {
      await skillMarketplace.open();
    } catch {
      setConnectionFailed(true);
    } finally {
      setOpening(false);
    }
  };

  /** Opens the directory containing a completed archive without launching the ZIP itself. */
  const openDownloadDirectory = async (archivePath: string) => {
    if (locationActions.kind !== "supported") return;
    const lastSeparator = Math.max(archivePath.lastIndexOf("/"), archivePath.lastIndexOf("\\"));
    const directoryPath = lastSeparator > 0 ? archivePath.slice(0, lastSeparator) : archivePath;
    try {
      await locationActions.open("explorer", directoryPath);
    } catch {
      toast.error(t("settings.skills.marketplaceOpenFolderFailed"));
    }
  };

  const unsupported = skillMarketplace.kind === "unsupported";

  return (
    <section className="rounded-lg border border-border bg-muted/20 p-4" aria-labelledby="skill-marketplace-title">
      <div className="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
        <div className="flex min-w-0 items-start gap-3">
          <div className="flex size-9 shrink-0 items-center justify-center rounded-md border border-border bg-background text-muted-foreground">
            <IconShoppingBag className="size-4" aria-hidden="true" />
          </div>
          <div className="min-w-0">
            <h3 id="skill-marketplace-title" className="text-sm font-medium">
              {t("settings.skills.marketplaceTitle")}
            </h3>
            <p className="mt-1 text-xs leading-5 text-muted-foreground">
              {t("settings.skills.marketplaceDescription")}
            </p>
          </div>
        </div>
        <Button
          type="button"
          variant="secondary"
          size="sm"
          className="shrink-0"
          disabled={unsupported || opening}
          onClick={() => void openMarketplace()}
        >
          <IconExternalLink aria-hidden="true" />
          {opening
            ? t("settings.skills.marketplaceOpening")
            : t("settings.skills.marketplaceOpen")}
        </Button>
      </div>

      <MarketplaceStatus
        status={status}
        unsupported={unsupported}
        connectionFailed={connectionFailed}
        canOpenDownloadDirectory={locationActions.kind === "supported"}
        onOpenDownloadDirectory={openDownloadDirectory}
      />
    </section>
  );
}

/** Renders one accessible status region without confusing unsupported hosts with failures. */
function MarketplaceStatus({
  status,
  unsupported,
  connectionFailed,
  canOpenDownloadDirectory,
  onOpenDownloadDirectory,
}: {
  status: SkillMarketplaceStatus | null;
  unsupported: boolean;
  connectionFailed: boolean;
  canOpenDownloadDirectory: boolean;
  onOpenDownloadDirectory: (archivePath: string) => Promise<void>;
}) {
  const { t } = useTranslation();

  if (unsupported) {
    return (
      <p className="mt-3 text-xs text-muted-foreground" role="status">
        {t("settings.skills.marketplaceUnsupported")}
      </p>
    );
  }
  if (connectionFailed) {
    return (
      <p className="mt-3 text-xs text-destructive" role="alert">
        {t("settings.skills.marketplaceConnectionFailed")}
      </p>
    );
  }
  if (status === null) return null;

  if (status.status === "downloading") {
    return (
      <p className="mt-3 text-xs text-muted-foreground" role="status">
        {t("settings.skills.marketplaceDownloading", { fileName: status.fileName })}
      </p>
    );
  }
  if (status.status === "failed") {
    return (
      <p className="mt-3 text-xs text-destructive" role="alert">
        {t("settings.skills.marketplaceDownloadFailed")}
      </p>
    );
  }

  return (
    <div className="mt-3 space-y-1 text-xs" role="status">
      <Button
        type="button"
        variant="link"
        size="sm"
        className="h-auto gap-1 p-0 text-xs"
        disabled={!canOpenDownloadDirectory}
        title={status.archivePath}
        onClick={() => void onOpenDownloadDirectory(status.archivePath)}
      >
        <IconFolderOpen className="size-3.5" aria-hidden="true" />
        {t("settings.skills.marketplaceSavedTo")}
      </Button>
    </div>
  );
}
