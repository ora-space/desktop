import { useState } from "react";
import { useTranslation } from "react-i18next";
import { IconHistory, IconRestore, IconTrash } from "@tabler/icons-react";
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
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@ora/ui";
import type { MockWorkflowVersion } from "@ora/workflow-mock";

interface WorkflowVersionHistoryProps {
  versions: MockWorkflowVersion[];
  previewedVersion: MockWorkflowVersion | null;
  /** Formatted last-edit time of the draft (workflow_snapshots.updated_at). */
  draftUpdatedAt?: string;
  onPreviewVersion: (version: MockWorkflowVersion | null) => void;
  onRestoreVersion: (version: MockWorkflowVersion) => void;
  onDeleteVersion: (version: MockWorkflowVersion) => void;
}

/** Provides a published-version picker backed by the persisted workflow version API. */
export function WorkflowVersionHistory({
  versions,
  previewedVersion,
  draftUpdatedAt,
  onPreviewVersion,
  onRestoreVersion,
  onDeleteVersion,
}: WorkflowVersionHistoryProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<MockWorkflowVersion | null>(null);

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
          </div>
          <div className="space-y-1 p-2">
            <VersionItem
              selected={previewedVersion === null}
              title={t("settings.workflow.currentDraft")}
              subtitle={draftUpdatedAt !== undefined
                ? `${t("settings.workflow.editableDraft")} · ${draftUpdatedAt}`
                : t("settings.workflow.editableDraft")}
              onClick={() => onPreviewVersion(null)}
            />
            {versions.map((version) => (
              <div key={version.version} className="group relative">
                <VersionItem
                  selected={previewedVersion?.version === version.version}
                  title={version.version}
                  subtitle={`${t("settings.workflow.publishedVersion")} · ${version.createdAt}`}
                  onClick={() => onPreviewVersion(version)}
                />
                <button
                  type="button"
                  aria-label={t("settings.workflow.deleteVersion", { version: version.createdAt })}
                  onClick={() => setDeleteTarget(version)}
                  className="absolute right-1.5 top-1/2 flex size-6 -translate-y-1/2 items-center justify-center rounded-md text-muted-foreground opacity-0 outline-none transition-opacity hover:bg-destructive/10 hover:text-destructive focus-visible:opacity-100 focus-visible:ring-2 focus-visible:ring-ring group-hover:opacity-100 group-focus-within:opacity-100"
                >
                  <IconTrash className="size-3.5" />
                </button>
              </div>
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
      <AlertDialog
        open={deleteTarget !== null}
        onOpenChange={(next) => {
          if (!next) {
            setDeleteTarget(null);
          }
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t("settings.workflow.deleteVersionConfirmTitle")}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t("settings.workflow.deleteVersionConfirmDescription")}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              onClick={() => {
                if (deleteTarget !== null) {
                  onDeleteVersion(deleteTarget);
                  setDeleteTarget(null);
                }
              }}
            >
              <IconTrash />
              {t("common.delete")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
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
      className={`w-full rounded-md px-2.5 py-2 pr-8 text-left transition-colors ${selected
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
