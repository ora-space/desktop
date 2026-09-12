import { useState } from "react";
import { useTranslation } from "react-i18next";
import { IconPlus, IconTrash } from "@tabler/icons-react";
import {
  Button,
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  Popover,
  PopoverContent,
  PopoverTrigger,
  Switch,
} from "@ora/ui";
import type {
  WorkflowAgentConfig,
  WorkflowMcpChoice,
} from "@ora/workflow-mock";
import type { WorkflowMcpCatalogStatus } from "./mcp-catalog";

/** Edits node-local intent while preserving bindings whose installed plugin is unavailable. */
export function WorkflowMcpFields({
  config,
  choices,
  catalog,
  onChange,
}: {
  config: WorkflowAgentConfig;
  choices: WorkflowMcpChoice[];
  catalog?: WorkflowMcpCatalogStatus;
  onChange: (config: WorkflowAgentConfig) => void;
}) {
  const { t } = useTranslation();
  const [mcpPickerOpen, setMcpPickerOpen] = useState(false);
  const configuredMcpIds = new Set(config.mcps.map((mcp) => mcp.mcpId));
  const availableMcps = choices.filter(
    (mcp) => !configuredMcpIds.has(mcp.value),
  );
  const enabledMcpCount = config.mcps.filter((mcp) => mcp.enabled).length;
  /** Adds a new MCP in its enabled state, preserving configuration order. */
  function addMcp(mcpId: string): void {
    if (config.mcps.some((mcp) => mcp.mcpId === mcpId)) return;
    onChange({
      ...config,
      mcps: [...config.mcps, { mcpId, enabled: true }],
    });
    setMcpPickerOpen(false);
  }

  /** Updates only the enabled state of a configured MCP. */
  function setMcpEnabled(mcpId: string, enabled: boolean): void {
    onChange({
      ...config,
      mcps: config.mcps.map((mcp) =>
        mcp.mcpId === mcpId ? { ...mcp, enabled } : mcp,
      ),
    });
  }

  /** Removes a configured MCP without affecting the remaining selection order. */
  function removeMcp(mcpId: string): void {
    onChange({
      ...config,
      mcps: config.mcps.filter((mcp) => mcp.mcpId !== mcpId),
    });
  }

  return (
    <fieldset className="min-w-0 space-y-2">
      <div className="flex min-w-0 flex-wrap items-center justify-between gap-2">
        <legend className="min-w-0 text-[11px] font-medium">
          {t("settings.workflow.field.mcps")}
        </legend>
        <div className="flex shrink-0 items-center gap-1">
          <span className="whitespace-nowrap text-[10px] text-muted-foreground">
            {t("settings.workflow.enabledMcpCount", {
              enabled: enabledMcpCount,
              total: config.mcps.length,
            })}
          </span>
          <Popover open={mcpPickerOpen} onOpenChange={setMcpPickerOpen}>
            <PopoverTrigger
              render={
                <Button
                  id="workflow-add-mcp"
                  type="button"
                  variant="ghost"
                  size="icon-sm"
                  disabled={
                    choices.length === 0 ||
                    catalog?.isLoading === true ||
                    catalog?.isError === true
                  }
                  aria-label={t("settings.workflow.addMcp")}
                />
              }
            >
              <IconPlus />
            </PopoverTrigger>
            <PopoverContent align="end" className="w-72 p-0">
              <Command>
                <CommandInput
                  aria-label={t("settings.workflow.searchAvailableMcps")}
                  placeholder={t("settings.workflow.searchAvailableMcps")}
                  className="text-sm"
                />
                <CommandList className="max-h-60">
                  <CommandEmpty className="py-6 text-center text-xs">
                    {t("settings.workflow.noAvailableMcps")}
                  </CommandEmpty>
                  <CommandGroup>
                    {availableMcps.map((mcp) => (
                      <CommandItem
                        key={mcp.value}
                        aria-label={`${mcp.label} ${mcp.value}`}
                        value={`${mcp.label} ${mcp.value}`}
                        onSelect={() => addMcp(mcp.value)}
                      >
                        <span className="min-w-0">
                          <span className="block truncate">{mcp.label}</span>
                          <span className="block truncate text-xs text-muted-foreground">
                            {mcp.value}
                          </span>
                          {mcp.unavailableReason && (
                            <span className="block text-xs text-destructive">
                              {t(
                                `settings.workflow.mcp.${mcp.unavailableReason}`,
                              )}
                            </span>
                          )}
                        </span>
                      </CommandItem>
                    ))}
                  </CommandGroup>
                </CommandList>
              </Command>
            </PopoverContent>
          </Popover>
        </div>
      </div>
      <div className="min-w-0 divide-y overflow-hidden rounded-md border border-border">
        {catalog?.isLoading && (
          <p role="status" className="p-2.5 text-xs">
            {t("settings.workflow.mcp.loading")}
          </p>
        )}
        {catalog?.isError && (
          <div role="alert" className="p-2.5 text-xs">
            <p>{t("settings.workflow.mcp.loadError")}</p>
            <Button
              type="button"
              variant="ghost"
              size="sm"
              onClick={catalog.onRetry}
            >
              {t("settings.workflow.mcp.retry")}
            </Button>
          </div>
        )}
        {!catalog?.isLoading && !catalog?.isError && choices.length === 0 && (
          <p className="p-2.5 text-xs text-muted-foreground">
            {t("settings.workflow.mcp.installHint")}
          </p>
        )}
        {config.mcps.map((configuredMcp) => {
          const installed = choices.find(
            (candidate) => candidate.value === configuredMcp.mcpId,
          );
          const mcp = installed ?? {
            value: configuredMcp.mcpId,
            label: configuredMcp.mcpId,
          };
          const reason =
            catalog?.isLoading || catalog?.isError
              ? undefined
              : (installed?.unavailableReason ??
                (installed === undefined ? "missing" : undefined));
          return (
            <div
              key={configuredMcp.mcpId}
              className="grid min-w-0 grid-cols-[minmax(0,1fr)_auto_auto] items-center gap-2 px-2.5 py-2"
            >
              <div className="min-w-0 text-xs">
                <span className="block truncate">{mcp.label}</span>
                <span className="block truncate text-muted-foreground">
                  {mcp.value}
                </span>
                {reason && (
                  <span className="block text-destructive">
                    {t(`settings.workflow.mcp.${reason}`)}
                  </span>
                )}
              </div>
              <Switch
                size="sm"
                className="shrink-0 data-checked:bg-blue-600 hover:data-checked:bg-blue-700"
                checked={configuredMcp.enabled}
                aria-label={t("settings.workflow.toggleMcp", {
                  name: mcp.label,
                })}
                onCheckedChange={(enabled) =>
                  setMcpEnabled(configuredMcp.mcpId, enabled)
                }
              />
              <Button
                type="button"
                variant="ghost"
                size="icon-sm"
                className="shrink-0 text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
                aria-label={t("settings.workflow.removeMcp", {
                  name: mcp.label,
                })}
                onClick={() => removeMcp(configuredMcp.mcpId)}
              >
                <IconTrash />
              </Button>
            </div>
          );
        })}
        {config.mcps.length === 0 && (
          <p className="px-2.5 py-3 text-xs text-muted-foreground">
            {t("settings.workflow.noConfiguredMcps")}
          </p>
        )}
      </div>
    </fieldset>
  );
}
