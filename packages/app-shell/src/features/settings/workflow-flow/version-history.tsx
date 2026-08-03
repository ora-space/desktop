import { useState } from "react";
import { useTranslation } from "react-i18next";
import { IconHistory, IconRestore } from "@tabler/icons-react";
import { Button, Popover, PopoverContent, PopoverTrigger } from "@ora/ui";
import type { MockWorkflowVersion } from "@ora/workflow-mock";

interface WorkflowVersionHistoryProps {
  versions: MockWorkflowVersion[];
  previewedVersion: MockWorkflowVersion | null;
  onPreviewVersion: (version: MockWorkflowVersion | null) => void;
  onRestoreVersion: (version: MockWorkflowVersion) => void;
}

/** Provides a mock published-version picker while the persisted workflow API is not yet available. */
export function WorkflowVersionHistory({
  versions,
  previewedVersion,
  onPreviewVersion,
  onRestoreVersion,
}: WorkflowVersionHistoryProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);

  return (
    <div className="absolute right-48 top-3 z-40">
      <Popover open={open} onOpenChange={setOpen}>
        <PopoverTrigger
          render={
            <Button
              type="button"
              variant="outline"
              size="icon-sm"
              aria-label={t("settings.workflow.versionHistory")}
            />
          }
        >
          <IconHistory />
        </PopoverTrigger>
        <PopoverContent align="end" className="w-80 p-0">
          <div className="flex items-center justify-between border-b border-border px-3 py-2.5">
            <h3 className="text-sm font-semibold">{t("settings.workflow.versionHistory")}</h3>
            <span className="text-[10px] text-muted-foreground">{t("settings.workflow.mockVersionHistory")}</span>
          </div>
          <div className="space-y-1 p-2">
            <VersionItem
              selected={previewedVersion === null}
              title={t("settings.workflow.currentDraft")}
              subtitle={t("settings.workflow.editableDraft")}
              onClick={() => onPreviewVersion(null)}
            />
            {versions.map((version) => (
              <VersionItem
                key={version.version}
                selected={previewedVersion?.version === version.version}
                title={version.createdAt}
                subtitle={t("settings.workflow.publishedVersion")}
                onClick={() => onPreviewVersion(version)}
              />
            ))}
          </div>
          {previewedVersion !== null && (
            <div className="border-t border-border p-2">
              <Button
                type="button"
                className="w-full"
                onClick={() => {
                  onRestoreVersion(previewedVersion);
                  setOpen(false);
                }}
              >
                <IconRestore />
                {t("settings.workflow.restoreVersion")}
              </Button>
            </div>
          )}
        </PopoverContent>
      </Popover>
    </div>
  );
}

/** Renders one historical graph choice without conflating preview with restoration. */
function VersionItem({
  selected,
  title,
  subtitle,
  onClick,
}: {
  selected: boolean;
  title: string;
  subtitle: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      aria-label={`${title} ${subtitle}`}
      className={`w-full rounded-md px-2.5 py-2 text-left transition-colors ${selected
        ? "bg-primary/10 text-foreground"
        : "hover:bg-muted"
      }`}
      onClick={onClick}
    >
      <span className="block text-xs font-medium">{title}</span>
      <span className="mt-0.5 block text-[10px] text-muted-foreground">{subtitle}</span>
    </button>
  );
}
