import { useState } from "react";
import { useTranslation } from "react-i18next";
import type {
  AvailablePlugin,
  InstalledPlugin,
  InstallOutcome,
} from "@ora/contracts";
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  Breadcrumb,
  BreadcrumbItem,
  BreadcrumbLink,
  BreadcrumbList,
  BreadcrumbPage,
  BreadcrumbSeparator,
  Button,
  toast,
} from "@ora/ui";
import {
  IconArrowBigUpLines,
  IconDownload,
  IconLoader2,
  IconTrash,
} from "@tabler/icons-react";
import { localizeContractError } from "../../i18n/contract-error";
import { useContractErrorToast } from "../../i18n/use-contract-error-toast";
import { useInstallPlugin } from "../../state/hooks/use-install-plugin";
import { usePluginMutations } from "../../state/hooks/use-plugin-mutations";
import { usePluginReadme } from "../../state/hooks/use-plugin-readme";
import { useUpdatePlugin } from "../../state/hooks/use-update-plugin";
import { MarkdownDocument } from "../chat/markdown-message";
import { PluginDownloadProgress } from "./plugin-download-progress";
import { PluginLogo } from "./plugin-logo";

/** The marketplace detail page: breadcrumb back navigation plus the listing's rendered README. */
export function PluginReadmeView({
  plugin,
  installed,
  onBack,
}: {
  plugin: AvailablePlugin;
  installed: InstalledPlugin | undefined;
  onBack: () => void;
}) {
  const { t } = useTranslation();
  const readme = usePluginReadme(plugin.id);

  return (
    <div className="space-y-5">
      <Breadcrumb>
        <BreadcrumbList>
          <BreadcrumbItem>
            <BreadcrumbLink render={<button type="button" onClick={onBack} />}>
              {t("settings.plugins.title")}
            </BreadcrumbLink>
          </BreadcrumbItem>
          <BreadcrumbSeparator />
          <BreadcrumbItem>
            <BreadcrumbPage>{plugin.title || plugin.name}</BreadcrumbPage>
          </BreadcrumbItem>
        </BreadcrumbList>
      </Breadcrumb>

      <header className="flex flex-col gap-3 sm:flex-row sm:items-start">
        <span className="flex min-w-0 flex-1 items-start gap-3">
          <PluginLogo logo={plugin.logo} />
          <span className="min-w-0 flex-1">
            <span className="block text-lg font-semibold">
              {plugin.title || plugin.name}
            </span>
            <span className="mt-0.5 block truncate text-xs text-muted-foreground">
              {plugin.id} · {plugin.version} · {plugin.kind}
            </span>
          </span>
        </span>
        <PluginDetailAction plugin={plugin} installed={installed} />
      </header>

      {readme.isLoading ? (
        <p className="py-10 text-center text-sm text-muted-foreground">
          {t("settings.plugins.readmeLoading")}
        </p>
      ) : readme.error !== null ? (
        <p className="py-10 text-center text-sm text-destructive">
          {t("settings.plugins.readmeFailed")}
          <span className="mt-1 block text-muted-foreground">
            {localizeContractError(readme.error, t)}
          </span>
        </p>
      ) : readme.data === undefined || readme.data.readme === null ? (
        <p className="py-10 text-center text-sm text-muted-foreground">
          {t("settings.plugins.readmeEmpty")}
        </p>
      ) : (
        <article>
          <MarkdownDocument content={readme.data.readme} />
        </article>
      )}
    </div>
  );
}

/** Offers the one package action relevant to the listing's current installation state. */
function PluginDetailAction({
  plugin,
  installed,
}: {
  plugin: AvailablePlugin;
  installed: InstalledPlugin | undefined;
}) {
  const { t } = useTranslation();
  const showContractError = useContractErrorToast();
  const install = useInstallPlugin(plugin.id);
  const update = useUpdatePlugin(plugin.id);
  const mutations = usePluginMutations(
    plugin.id,
    installed?.kind === "agent" ? installed.id : undefined,
  );
  const [uninstallOpen, setUninstallOpen] = useState(false);
  const [deleteData, setDeleteData] = useState(true);
  const uninstalling = mutations.uninstall.isPending;
  const incompatible = plugin.compatibility === "incompatible";
  const hasUpdate =
    installed !== undefined && plugin.version !== installed.version;

  const failInstall = (cause: unknown) => {
    showContractError(cause, t("settings.plugins.installFailed"));
  };
  const succeedInstall = (response: { outcome: InstallOutcome }) => {
    toast.success(
      response.outcome.state === "installed_with_command_conflict"
        ? t("settings.plugins.installCommandConflict", {
            pluginId: response.outcome.conflictPluginId,
          })
        : t("settings.plugins.installSuccess"),
    );
  };
  const failUpdate = (cause: unknown) => {
    showContractError(cause, t("settings.plugins.updateFailed"));
  };
  const failUninstall = (cause: unknown) => {
    showContractError(cause, t("settings.plugins.uninstallFailed"));
  };

  if (install.isPending) {
    return (
      <Button disabled className="shrink-0 disabled:opacity-100">
        <PluginDownloadProgress
          progress={install.progress}
          label={t("settings.plugins.downloadProgress")}
        />
        {t("settings.plugins.installing")}
      </Button>
    );
  }
  if (update.isPending) {
    return (
      <Button disabled className="shrink-0 disabled:opacity-100">
        <PluginDownloadProgress
          progress={update.progress}
          label={t("settings.plugins.downloadProgress")}
        >
          <IconArrowBigUpLines className="size-3.5" />
        </PluginDownloadProgress>
        {t("settings.plugins.updating")}
      </Button>
    );
  }
  if (installed === undefined) {
    return (
      <Button
        variant="outline"
        className="shrink-0"
        disabled={incompatible}
        onClick={() =>
          install.mutate(
            {},
            { onError: failInstall, onSuccess: succeedInstall },
          )
        }
      >
        <IconDownload />
        {t("settings.plugins.install")}
      </Button>
    );
  }
  if (hasUpdate) {
    return (
      <Button
        className="shrink-0"
        onClick={() => update.mutate({}, { onError: failUpdate })}
      >
        <IconArrowBigUpLines />
        {t("settings.plugins.update")}
      </Button>
    );
  }

  return (
    <>
      <Button
        variant="outline"
        className="shrink-0 text-destructive hover:bg-destructive/10 hover:text-destructive"
        disabled={uninstalling}
        onClick={() => setUninstallOpen(true)}
      >
        {uninstalling ? (
          <IconLoader2 className="animate-spin" />
        ) : (
          <IconTrash />
        )}
        {t(
          uninstalling
            ? "settings.plugins.uninstalling"
            : "settings.plugins.uninstall",
        )}
      </Button>
      <AlertDialog
        open={uninstallOpen}
        onOpenChange={(open) => {
          setUninstallOpen(open);
          if (open) setDeleteData(true);
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t("settings.plugins.uninstallTitle", {
                name: plugin.title || plugin.name,
              })}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t("settings.plugins.uninstallDescription")}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <label className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={deleteData}
              onChange={(event) => setDeleteData(event.target.checked)}
            />
            {t("settings.plugins.deleteConfigurationData")}
          </label>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={uninstalling}>
              {t("common.cancel")}
            </AlertDialogCancel>
            <Button
              variant="destructive"
              disabled={uninstalling}
              onClick={() =>
                mutations.uninstall.mutate(deleteData ? "delete" : "retain", {
                  onError: failUninstall,
                  onSuccess: () => setUninstallOpen(false),
                })
              }
            >
              {t("settings.plugins.uninstall")}
            </Button>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}
