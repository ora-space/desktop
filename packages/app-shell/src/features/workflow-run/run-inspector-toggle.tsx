import { useTranslation } from "react-i18next";
import {
  IconLayoutSidebarRightCollapse,
  IconLayoutSidebarRightExpand,
} from "@tabler/icons-react";
import { Button, cn } from "@ora/ui";

export function RunInspectorToggle({
  open,
  onToggle,
}: {
  open: boolean;
  onToggle: () => void;
}) {
  const { t } = useTranslation();
  const label = t(
    open
      ? "workflowRun.inspector.collapse"
      : "workflowRun.theater.openInspector",
  );
  return (
    <Button
      type="button"
      variant="ghost"
      size="icon-sm"
      className={cn(
        "size-9 shrink-0 rounded-full border shadow-sm",
        open
          ? "border-primary/40 bg-primary/10 text-primary hover:bg-primary/15"
          : "border-border/80 bg-background hover:border-primary/30 hover:bg-primary/5",
      )}
      aria-label={label}
      title={label}
      aria-pressed={open}
      onPointerDown={(event) => event.stopPropagation()}
      onClick={onToggle}
    >
      {open ? (
        <IconLayoutSidebarRightCollapse className="size-4" />
      ) : (
        <IconLayoutSidebarRightExpand className="size-4" />
      )}
    </Button>
  );
}
