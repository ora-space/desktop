import type { ReactNode, KeyboardEvent } from "react";
import { Button, Popover, PopoverContent, PopoverTrigger, Textarea } from "@ora/ui";
import {
  IconCheck,
  IconChevronDown,
  IconGitCommit,
  IconUpload,
} from "@tabler/icons-react";
import { useTranslation } from "react-i18next";

interface TaskGitActionsProps {
  open: boolean;
  message: string;
  additions: number;
  deletions: number;
  pending: boolean;
  onOpenChange: (open: boolean) => void;
  onMessageChange: (message: string) => void;
  onCommit: () => void;
  onCommitAndPush: () => void;
  onPush: () => void;
}

/** Renders the task Git actions as one Codex-style popover instead of separate toolbar buttons. */
export function TaskGitActions({
  open,
  message,
  additions,
  deletions,
  pending,
  onOpenChange,
  onMessageChange,
  onCommit,
  onCommitAndPush,
  onPush,
}: TaskGitActionsProps) {
  const { t } = useTranslation();
  const canCommit = message.trim() !== "";

  const handleMessageKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if ((event.ctrlKey || event.metaKey) && event.key === "Enter" && canCommit && !pending) {
      event.preventDefault();
      onCommit();
    }
  };

  return (
    <Popover open={open} onOpenChange={(nextOpen) => !pending && onOpenChange(nextOpen)}>
      <PopoverTrigger
        render={
          <Button
            type="button"
            size="sm"
            variant={open ? "secondary" : "outline"}
            className="ora-diff-toolbar__commit-trigger h-8 gap-1.5 rounded-lg px-2.5 shadow-xs"
            aria-label={t("diff.gitActions")}
            aria-expanded={open}
          />
        }
      >
        <IconGitCommit />
        <span className="ora-diff-toolbar__commit-label">{t("diff.commit")}</span>
        <IconChevronDown
          className={`size-3.5 text-muted-foreground transition-transform ${open ? "rotate-180" : ""}`}
          aria-hidden="true"
        />
      </PopoverTrigger>
      <PopoverContent
        align="start"
        side="bottom"
        className="w-[min(20rem,calc(100vw-1rem))] gap-0 overflow-hidden p-0"
        aria-label={t("diff.gitActions")}
      >
        <div className="border-b border-border/70 p-3">
          <Textarea
            autoFocus
            rows={2}
            value={message}
            onChange={(event) => onMessageChange(event.target.value)}
            onKeyDown={handleMessageKeyDown}
            placeholder={t("diff.commitMessagePlaceholder")}
            aria-label={t("diff.commitMessage")}
            disabled={pending}
            className="min-h-16 resize-none bg-muted/35 text-xs shadow-none focus-visible:ring-1"
          />
          <div className="mt-2 flex items-center gap-1.5 rounded-md bg-muted/35 px-2 py-1.5 text-[11px] text-muted-foreground">
            <IconCheck className="size-3.5 shrink-0 text-emerald-600" aria-hidden="true" />
            <span className="min-w-0 flex-1 truncate">{t("diff.allChangesIncluded")}</span>
            <span className="shrink-0 font-mono text-emerald-600">+{additions}</span>
            <span className="shrink-0 font-mono text-red-600">-{deletions}</span>
          </div>
        </div>
        <div className="p-1.5">
          <GitActionRow
            icon={<IconGitCommit />}
            label={pending ? t("diff.committing") : t("diff.commit")}
            shortcut={t("diff.commitShortcut")}
            disabled={!canCommit || pending}
            onClick={onCommit}
          />
          <GitActionRow
            icon={<IconUpload />}
            label={t("diff.commitAndPush")}
            disabled={!canCommit || pending}
            onClick={onCommitAndPush}
          />
          <div className="my-1 border-t border-border/70" />
          <GitActionRow
            icon={<IconUpload />}
            label={pending ? t("diff.pushing") : t("diff.push")}
            disabled={pending}
            onClick={onPush}
          />
        </div>
      </PopoverContent>
    </Popover>
  );
}

interface GitActionRowProps {
  icon: ReactNode;
  label: string;
  shortcut?: string;
  disabled: boolean;
  onClick: () => void;
}

/** Keeps each Git command keyboard-friendly and visually aligned in the action menu. */
function GitActionRow({ icon, label, shortcut, disabled, onClick }: GitActionRowProps) {
  return (
    <Button
      type="button"
      size="sm"
      variant="ghost"
      className="h-8 w-full justify-start px-2 text-xs"
      disabled={disabled}
      onClick={onClick}
    >
      {icon}
      <span>{label}</span>
      {shortcut !== undefined && (
        <kbd aria-hidden="true" className="ml-auto text-[10px] text-muted-foreground">{shortcut}</kbd>
      )}
    </Button>
  );
}
