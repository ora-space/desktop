import type { RuntimeLogLevel } from "@ora/contracts";
import {
  DropdownMenuItem,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
} from "@ora/ui";
import { IconDownload, IconFileText, IconLoader2 } from "@tabler/icons-react";
import { useTranslation } from "react-i18next";
import { useContractErrorToast } from "../../i18n/use-contract-error-toast";
import { usePluginLogDownload } from "../../state/hooks/use-plugin-log-download";
import { usePluginLogLevel } from "../../state/hooks/use-plugin-log-level";

const LOG_LEVELS: RuntimeLogLevel[] = [
  "trace",
  "debug",
  "info",
  "warn",
  "error",
];

/**
 * Developer-only menu items for one installed plugin's host-owned log: the level submenu and,
 * when the host can export files, a download of the active log.
 *
 * The level is the floor the host persists into that plugin's own log file; it is independent
 * of the runtime log level in Developer settings and of every other plugin.
 */
export function PluginLogMenuItems({
  pluginId,
  displayName,
}: {
  pluginId: string;
  displayName: string;
}) {
  const { t } = useTranslation();
  const showContractError = useContractErrorToast();
  const { state, isLoading, isSaving, setLevel } = usePluginLogLevel(pluginId);
  const download = usePluginLogDownload(pluginId, displayName);
  const current = state?.level;

  return (
    <>
      <DropdownMenuSub>
        <DropdownMenuSubTrigger disabled={isLoading || state === undefined}>
          <IconFileText />
          <span className="flex-1">{t("settings.plugins.logLevel")}</span>
          {current !== undefined && (
            <span className="ml-2 text-xs text-muted-foreground">
              {t(`settings.plugins.logLevel.${current}`)}
            </span>
          )}
        </DropdownMenuSubTrigger>
        <DropdownMenuSubContent className="w-40">
          <DropdownMenuRadioGroup
            value={current ?? ""}
            onValueChange={(value) =>
              setLevel(value as RuntimeLogLevel, {
                onError: (cause) =>
                  showContractError(
                    cause,
                    t("settings.plugins.logLevelUpdateFailed"),
                  ),
              })
            }
          >
            {LOG_LEVELS.map((level) => (
              <DropdownMenuRadioItem
                key={level}
                value={level}
                disabled={isSaving}
              >
                {t(`settings.plugins.logLevel.${level}`)}
              </DropdownMenuRadioItem>
            ))}
          </DropdownMenuRadioGroup>
        </DropdownMenuSubContent>
      </DropdownMenuSub>
      {download !== undefined && (
        <DropdownMenuItem
          disabled={download.isDownloading}
          onClick={() => void download.download()}
        >
          {download.isDownloading ? (
            <IconLoader2 className="animate-spin" />
          ) : (
            <IconDownload />
          )}
          {t("settings.plugins.downloadLog")}
        </DropdownMenuItem>
      )}
      <DropdownMenuSeparator />
    </>
  );
}
