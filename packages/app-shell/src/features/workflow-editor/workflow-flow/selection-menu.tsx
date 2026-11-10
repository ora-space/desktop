import { useEffect } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { cn } from "@ora/ui";

interface WorkflowSelectionMenuProps {
  point: { x: number; y: number };
  canCopy: boolean;
  canPaste: boolean;
  canDelete: boolean;
  onCopy: () => void;
  onPaste: () => void;
  onDuplicate: () => void;
  onDelete: () => void;
  onClose: () => void;
}

/** Right-click actions for the selected workflow nodes. */
export function WorkflowSelectionMenu({
  point,
  canCopy,
  canPaste,
  canDelete,
  onCopy,
  onPaste,
  onDuplicate,
  onDelete,
  onClose,
}: WorkflowSelectionMenuProps) {
  const { t } = useTranslation();
  const modifier = /Mac|iPhone|iPad/i.test(navigator.platform) ? "⌘" : "Ctrl";
  useEffect(() => {
    function onPointerDown(event: PointerEvent): void {
      if (
        event.target instanceof Element &&
        event.target.closest("[data-workflow-selection-menu]") !== null
      ) {
        return;
      }
      onClose();
    }
    function onKeyDown(event: KeyboardEvent): void {
      if (event.key === "Escape") {
        onClose();
      }
    }
    window.addEventListener("pointerdown", onPointerDown);
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("pointerdown", onPointerDown);
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [onClose]);

  return createPortal(
    <div
      role="menu"
      data-workflow-selection-menu=""
      className="fixed z-[80] min-w-36 rounded-lg bg-popover p-1 text-popover-foreground shadow-md ring-1 ring-foreground/10"
      style={{ left: point.x, top: point.y }}
    >
      <SelectionMenuItem
        label={t("settings.workflow.copyNodes")}
        shortcut={`${modifier}+C`}
        disabled={!canCopy}
        onSelect={() => {
          onCopy();
          onClose();
        }}
      />
      <SelectionMenuItem
        label={t("settings.workflow.pasteNodes")}
        shortcut={`${modifier}+V`}
        disabled={!canPaste}
        onSelect={() => {
          onPaste();
          onClose();
        }}
      />
      <SelectionMenuItem
        label={t("settings.workflow.duplicateNodes")}
        shortcut={`${modifier}+D`}
        disabled={!canCopy}
        onSelect={() => {
          onDuplicate();
          onClose();
        }}
      />
      <div className="-mx-1 my-1 h-px bg-border" role="separator" />
      <SelectionMenuItem
        label={t("settings.workflow.deleteNodes")}
        shortcut="Delete"
        disabled={!canDelete}
        destructive
        onSelect={() => {
          onDelete();
          onClose();
        }}
      />
    </div>,
    document.body,
  );
}

function SelectionMenuItem({
  label,
  shortcut,
  disabled,
  destructive = false,
  onSelect,
}: {
  label: string;
  shortcut: string;
  disabled: boolean;
  destructive?: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      role="menuitem"
      className={cn(
        "flex w-full items-center gap-3 rounded-md px-1.5 py-1 text-left text-sm outline-hidden select-none hover:bg-accent hover:text-accent-foreground focus:bg-accent focus:text-accent-foreground disabled:pointer-events-none disabled:opacity-50",
        destructive &&
          "text-destructive hover:bg-destructive/10 hover:text-destructive focus:bg-destructive/10 focus:text-destructive",
      )}
      disabled={disabled}
      onClick={onSelect}
    >
      <span>{label}</span>
      <span className="ml-auto text-xs tracking-widest text-muted-foreground">
        {shortcut}
      </span>
    </button>
  );
}
