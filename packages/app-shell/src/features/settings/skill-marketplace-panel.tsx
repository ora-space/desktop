import { useEffect, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Badge, Button, toast } from "@ora/ui";
import {
  IconExternalLink,
  IconFolderOpen,
  IconShoppingBag,
  IconWorld,
} from "@tabler/icons-react";
import {
  usePlatform,
  type SkillMarketplaceProvider,
  type SkillMarketplaceStatus,
} from "../../platform";

/** Opens provider-specific marketplaces and keeps their latest download status visible. */
export function SkillMarketplacePanel() {
  const { t } = useTranslation();
  const { locationActions, skillMarketplace } = usePlatform();
  const [status, setStatus] = useState<SkillMarketplaceStatus | null>(null);
  const [openingProvider, setOpeningProvider] =
    useState<SkillMarketplaceProvider | null>(null);
  const [failedProvider, setFailedProvider] =
    useState<SkillMarketplaceProvider | null>(null);

  useEffect(() => {
    let disposed = false;
    let unsubscribe: (() => void) | undefined;
    void skillMarketplace
      .onStatus((nextStatus) => {
        if (disposed) return;

        setStatus(nextStatus);
      })
      .then((stop) => {
        if (disposed) stop();
        else unsubscribe = stop;
      })
      .catch(() => {
        if (!disposed) setFailedProvider("skillHub");
      });

    return () => {
      disposed = true;
      unsubscribe?.();
    };
  }, [skillMarketplace]);

  /** Opens one native marketplace WebView while preventing duplicate actions per surface. */
  const openMarketplace = async (provider: SkillMarketplaceProvider) => {
    setOpeningProvider(provider);
    setFailedProvider(null);
    try {
      await skillMarketplace.open(provider);
    } catch {
      setFailedProvider(provider);
    } finally {
      setOpeningProvider(null);
    }
  };

  /** Opens the directory containing a completed archive without launching the ZIP itself. */
  const openDownloadDirectory = async (archivePath: string) => {
    const lastSeparator = Math.max(
      archivePath.lastIndexOf("/"),
      archivePath.lastIndexOf("\\"),
    );
    const directoryPath =
      lastSeparator > 0 ? archivePath.slice(0, lastSeparator) : archivePath;
    try {
      await locationActions.open("explorer", directoryPath);
    } catch {
      toast.error(t("settings.skills.marketplaceOpenFolderFailed"));
    }
  };

  return (
    <section
      className="rounded-lg border border-border bg-muted/20 p-4"
      aria-labelledby="skill-marketplace-title"
    >
      <div className="flex min-w-0 items-start gap-3">
        <div className="flex size-9 shrink-0 items-center justify-center rounded-md border border-border bg-background text-muted-foreground">
          <IconShoppingBag className="size-4" aria-hidden="true" />
        </div>
        <div className="min-w-0">
          <h3 id="skill-marketplace-title" className="text-sm font-medium">
            {t("settings.skills.marketplacesTitle")}
          </h3>
          <p className="mt-1 text-xs leading-5 text-muted-foreground">
            {t("settings.skills.marketplacesDescription")}
          </p>
        </div>
      </div>

      <div className="mt-4 grid gap-3 lg:grid-cols-2">
        <MarketplaceCard
          icon={IconShoppingBag}
          title={t("settings.skills.marketplaceTitle")}
          description={t("settings.skills.marketplaceDescription")}
          badge={t("settings.skills.marketplacePublicBadge")}
          action={
            <Button
              type="button"
              variant="secondary"
              size="sm"
              disabled={openingProvider !== null}
              onClick={() => void openMarketplace("skillHub")}
            >
              <IconExternalLink aria-hidden="true" />
              {openingProvider === "skillHub"
                ? t("settings.skills.marketplaceOpening")
                : t("settings.skills.marketplaceOpen")}
            </Button>
          }
        >
          <MarketplaceStatus
            status={status?.provider === "skillHub" ? status : null}
            connectionFailed={failedProvider === "skillHub"}
            onOpenDownloadDirectory={openDownloadDirectory}
          />
        </MarketplaceCard>

        <MarketplaceCard
          icon={IconWorld}
          title={t("settings.skills.huaweiTitle")}
          description={t("settings.skills.huaweiDescription")}
          badge={t("settings.skills.huaweiBadge")}
          action={
            <Button
              type="button"
              variant="secondary"
              size="sm"
              disabled={openingProvider !== null}
              onClick={() => void openMarketplace("huaweiAgentCenter")}
            >
              <IconExternalLink aria-hidden="true" />
              {openingProvider === "huaweiAgentCenter"
                ? t("settings.skills.marketplaceOpening")
                : t("settings.skills.huaweiOpen")}
            </Button>
          }
        >
          <MarketplaceStatus
            status={status?.provider === "huaweiAgentCenter" ? status : null}
            connectionFailed={failedProvider === "huaweiAgentCenter"}
            onOpenDownloadDirectory={openDownloadDirectory}
          />
        </MarketplaceCard>
      </div>
    </section>
  );
}

/** Renders one marketplace summary card with a provider-owned action and optional status. */
function MarketplaceCard({
  icon: Icon,
  title,
  description,
  badge,
  action,
  children,
}: {
  icon: typeof IconShoppingBag;
  title: string;
  description: string;
  badge: string;
  action: ReactNode;
  children?: ReactNode;
}) {
  return (
    <article className="flex min-h-44 flex-col rounded-lg border border-border bg-background p-3">
      <div className="flex items-start justify-between gap-3">
        <div className="flex min-w-0 items-start gap-3">
          <div className="flex size-8 shrink-0 items-center justify-center rounded-md bg-muted/50 text-muted-foreground">
            <Icon className="size-4" aria-hidden="true" />
          </div>
          <div className="min-w-0">
            <div className="flex flex-wrap items-center gap-2">
              <h4 className="text-sm font-medium">{title}</h4>
              <Badge variant="outline">{badge}</Badge>
            </div>
            <p className="mt-1 text-xs leading-5 text-muted-foreground">
              {description}
            </p>
          </div>
        </div>
      </div>
      <div className="mt-auto pt-4">{action}</div>
      {children}
    </article>
  );
}

/** Renders one accessible marketplace status region. */
function MarketplaceStatus({
  status,
  connectionFailed,
  onOpenDownloadDirectory,
}: {
  status: SkillMarketplaceStatus | null;
  connectionFailed: boolean;
  onOpenDownloadDirectory: (archivePath: string) => Promise<void>;
}) {
  const { t } = useTranslation();

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
        {t("settings.skills.marketplaceDownloading", {
          fileName: status.fileName,
        })}
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
        title={status.archivePath}
        onClick={() => void onOpenDownloadDirectory(status.archivePath)}
      >
        <IconFolderOpen className="size-3.5" aria-hidden="true" />
        {t("settings.skills.marketplaceSavedTo")}
      </Button>
    </div>
  );
}
