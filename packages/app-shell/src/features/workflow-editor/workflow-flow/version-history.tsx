import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  IconCircleCheck,
  IconHistory,
  IconSearch,
  IconTrash,
  IconVersions,
} from "@tabler/icons-react";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  Badge,
  Button,
  Input,
  Popover,
  PopoverContent,
  PopoverTrigger,
  cn,
} from "@ora/ui";
import type { MockWorkflowVersion } from "@ora/workflow-mock";

interface WorkflowVersionHistoryProps {
  versions: MockWorkflowVersion[];
  previewedVersion: MockWorkflowVersion | null;
  /** Version string of the workflow's currently active published snapshot, if any. */
  activeVersion: string | null;
  /** Formatted last-edit time of the draft (workflow_snapshots.updated_at). */
  draftUpdatedAt?: string;
  onPreviewVersion: (version: MockWorkflowVersion | null) => void;
  /** Makes the previewed published snapshot the active run target and loads it into the draft. */
  onActivateVersion: (version: MockWorkflowVersion) => void;
  /** Opens the same publish flow as the header, freezing the current draft. */
  onPublishDraft: () => void;
  onDeleteVersion: (version: MockWorkflowVersion) => void;
}

/** Provides a published-version picker backed by the persisted workflow version API. */
export function WorkflowVersionHistory({
  versions,
  previewedVersion,
  activeVersion,
  draftUpdatedAt,
  onPreviewVersion,
  onActivateVersion,
  onPublishDraft,
  onDeleteVersion,
}: WorkflowVersionHistoryProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [deleteTarget, setDeleteTarget] = useState<MockWorkflowVersion | null>(
    null,
  );

  const normalizedQuery = query.trim().toLowerCase();
  const draftTitle = t("settings.workflow.currentDraft");
  const draftSubtitle =
    draftUpdatedAt !== undefined
      ? `${t("settings.workflow.editableDraft")} · ${draftUpdatedAt}`
      : t("settings.workflow.editableDraft");
  const showDraft =
    normalizedQuery === "" ||
    draftTitle.toLowerCase().includes(normalizedQuery) ||
    draftSubtitle.toLowerCase().includes(normalizedQuery);

  const visibleVersions = useMemo(() => {
    if (normalizedQuery === "") {
      return versions;
    }
    return versions.filter((version) => {
      const haystack = `${version.version} ${version.createdAt}`.toLowerCase();
      return haystack.includes(normalizedQuery);
    });
  }, [normalizedQuery, versions]);

  const previewIsActive =
    previewedVersion !== null &&
    activeVersion !== null &&
    previewedVersion.version === activeVersion;
  // Same muted caption as the header autosave line: the clock stays an icon
  // control, and this label only states unpublished / active / preview.
  const statusLabel =
    previewedVersion !== null
      ? t("settings.workflow.previewingVersionBanner", {
          version: previewedVersion.version,
        })
      : activeVersion === null
        ? t("settings.workflow.unpublished")
        : t("settings.workflow.activeVersionChip", { version: activeVersion });

  return (
    <div className="flex min-w-0 shrink-0 items-center gap-2">
      <p
        className="max-w-56 shrink-0 truncate text-right text-[10px] leading-4 text-muted-foreground"
        title={statusLabel}
      >
        {statusLabel}
      </p>
      {previewedVersion !== null ? (
        <Button
          type="button"
          variant="ghost"
          size="xs"
          className="h-auto px-0 text-[10px] leading-4 text-muted-foreground hover:bg-transparent hover:text-foreground"
          onClick={() => onPreviewVersion(null)}
        >
          {t("settings.workflow.returnToDraft")}
        </Button>
      ) : null}
      <Popover
        open={open}
        onOpenChange={(next) => {
          setOpen(next);
          if (!next) {
            setQuery("");
          }
        }}
      >
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
            <h3 className="text-sm font-semibold">
              {t("settings.workflow.versionHistory")}
            </h3>
          </div>
          <div className="border-b border-border px-2 py-2">
            <div className="relative">
              <IconSearch className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" />
              <Input
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                aria-label={t("settings.workflow.searchVersions")}
                placeholder={t("settings.workflow.searchVersions")}
                className="h-8 pl-8 text-xs"
              />
            </div>
          </div>
          <div className="max-h-72 space-y-1 overflow-y-auto p-2">
            {showDraft ? (
              <div className="group relative">
                <VersionItem
                  selected={previewedVersion === null}
                  title={draftTitle}
                  subtitle={draftSubtitle}
                  trailingAction
                  onClick={() => onPreviewVersion(null)}
                />
                <button
                  type="button"
                  aria-label={t("settings.workflow.publishDraft")}
                  onClick={(event) => {
                    event.stopPropagation();
                    setOpen(false);
                    onPublishDraft();
                  }}
                  className="absolute right-1.5 top-1/2 flex size-6 -translate-y-1/2 items-center justify-center rounded-md text-muted-foreground opacity-0 outline-none transition-opacity hover:bg-muted hover:text-foreground focus-visible:opacity-100 focus-visible:ring-2 focus-visible:ring-ring group-hover:opacity-100 group-focus-within:opacity-100"
                >
                  <IconVersions className="size-3.5" />
                </button>
              </div>
            ) : null}
            {visibleVersions.map((version) => {
              const isActive =
                activeVersion !== null && version.version === activeVersion;
              return (
                <div key={version.version} className="group relative">
                  <VersionItem
                    selected={previewedVersion?.version === version.version}
                    title={version.version}
                    subtitle={
                      isActive
                        ? `${t("settings.workflow.activeVersion")} · ${version.createdAt}`
                        : `${t("settings.workflow.publishedVersion")} · ${version.createdAt}`
                    }
                    badge={
                      isActive
                        ? t("settings.workflow.activeVersion")
                        : undefined
                    }
                    trailingAction={!isActive}
                    onClick={() => onPreviewVersion(version)}
                  />
                  {!isActive ? (
                    <button
                      type="button"
                      aria-label={t("settings.workflow.deleteVersion", {
                        version: version.version,
                      })}
                      onClick={() => setDeleteTarget(version)}
                      className="absolute right-1.5 top-1/2 flex size-6 -translate-y-1/2 items-center justify-center rounded-md text-muted-foreground opacity-0 outline-none transition-opacity hover:bg-destructive/10 hover:text-destructive focus-visible:opacity-100 focus-visible:ring-2 focus-visible:ring-ring group-hover:opacity-100 group-focus-within:opacity-100"
                    >
                      <IconTrash className="size-3.5" />
                    </button>
                  ) : null}
                </div>
              );
            })}
            {!showDraft && visibleVersions.length === 0 ? (
              <p className="px-2.5 py-3 text-center text-[11px] text-muted-foreground">
                {t("settings.workflow.noMatchingVersions")}
              </p>
            ) : null}
          </div>
          {previewedVersion !== null ? (
            <div className="border-t border-border p-2">
              {previewIsActive ? (
                <p className="px-1 py-1.5 text-center text-[11px] leading-4 text-muted-foreground">
                  {t("settings.workflow.previewingActiveVersion")}
                </p>
              ) : (
                <Button
                  type="button"
                  className="w-full"
                  onClick={() => {
                    onActivateVersion(previewedVersion);
                    setOpen(false);
                  }}
                >
                  <IconCircleCheck />
                  {t("settings.workflow.activateVersion")}
                </Button>
              )}
            </div>
          ) : null}
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

/** Renders one historical graph choice without conflating preview with activation. */
function VersionItem({
  selected,
  title,
  subtitle,
  badge,
  trailingAction = false,
  onClick,
}: {
  selected: boolean;
  title: string;
  subtitle: string;
  badge?: string;
  trailingAction?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      aria-label={`${title} ${subtitle}${badge !== undefined ? ` ${badge}` : ""}`}
      className={cn(
        "w-full rounded-md px-2.5 py-2 text-left transition-colors",
        trailingAction ? "pr-8" : "pr-2.5",
        selected ? "bg-primary/10 text-foreground" : "hover:bg-muted",
      )}
      onClick={onClick}
    >
      <span className="flex min-w-0 items-center gap-1.5">
        <span className="min-w-0 truncate text-xs font-medium">{title}</span>
        {badge !== undefined ? (
          <Badge
            variant="secondary"
            className="h-4 shrink-0 px-1.5 text-[9px] font-medium"
          >
            {badge}
          </Badge>
        ) : null}
      </span>
      <span className="mt-0.5 block truncate text-[10px] text-muted-foreground">
        {subtitle}
      </span>
    </button>
  );
}
