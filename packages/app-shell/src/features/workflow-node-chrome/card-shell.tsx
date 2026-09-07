import type { CSSProperties, HTMLAttributes, ReactNode } from "react";
import { cn } from "@ora/ui";
import { WORKFLOW_NODE_WIDTH, type WorkflowNodeKind } from "@ora/workflow-mock";
import { getNodeMetadata } from "./metadata";

/** Visual density shared by editor cards and run overlays. */
export type WorkflowNodeCardDensity = "editor" | "run" | "stage" | "compact";

export interface WorkflowNodeCardShellProps extends Omit<
  HTMLAttributes<HTMLElement>,
  "title" | "children"
> {
  kind: WorkflowNodeKind;
  title: string;
  description: string;
  kindLabel: string;
  ariaLabel: string;
  density?: WorkflowNodeCardDensity;
  selected?: boolean;
  width?: number;
  /** Extra frame classes (e.g. run status rings) without forking the shell. */
  frameClassName?: string;
  style?: CSSProperties;
  /** Overlay on the kind icon (status dot, etc.). */
  iconAccessory?: ReactNode;
  /** Compact marker rendered immediately after the node title. */
  titleAccessory?: ReactNode;
  /** Trailing chips in the title row (status badge, etc.). */
  headerAccessory?: ReactNode;
  /** Far-right header control (delete in editor). */
  headerEnd?: ReactNode;
  /** Bottom bar (model/id in editor, metrics in run). */
  footer?: ReactNode;
  /** Replaces the default description block when set. */
  body?: ReactNode;
  /** Read-only details rendered at the full card width beneath the header. */
  details?: ReactNode;
  /** Layout classes for the full-width details slot. */
  detailsClassName?: string;
  targetHandle?: ReactNode;
  sourceHandle?: ReactNode;
}

const DENSITY: Record<
  WorkflowNodeCardDensity,
  {
    radius: string;
    iconBox: string;
    iconSize: string;
    title: string;
    description: string;
    headerPad: string;
    gap: string;
  }
> = {
  editor: {
    radius: "rounded-xl",
    iconBox: "size-8 rounded-lg",
    iconSize: "size-4",
    title: "font-sans text-sm font-bold",
    description:
      "mt-1 line-clamp-2 text-[10px] leading-4 text-muted-foreground",
    headerPad: "px-3 py-3",
    gap: "gap-2.5",
  },
  run: {
    radius: "rounded-xl",
    iconBox: "size-8 rounded-lg",
    iconSize: "size-4",
    title: "font-sans text-xs font-semibold",
    description:
      "mt-1 line-clamp-2 text-[10px] leading-4 text-muted-foreground",
    headerPad: "px-3 py-3",
    gap: "gap-2.5",
  },
  stage: {
    radius: "rounded-2xl",
    iconBox: "size-12 rounded-xl",
    iconSize: "size-5",
    title: "font-sans text-lg font-semibold tracking-[-0.02em]",
    description: "mt-2 text-sm leading-6 text-muted-foreground",
    headerPad: "p-6 pb-0",
    gap: "gap-4",
  },
  compact: {
    radius: "rounded-xl",
    iconBox: "size-8 rounded-lg",
    iconSize: "size-3.5",
    title: "font-sans text-sm font-semibold",
    description:
      "mt-1 line-clamp-2 text-[11px] leading-4 text-muted-foreground",
    headerPad: "px-3.5 py-3",
    gap: "gap-2.5",
  },
};

/**
 * Shared workflow node chrome for settings + run UIs.
 * Callers pass Handles / actions through slots so interaction stays local.
 */
export function WorkflowNodeCardShell({
  kind,
  title,
  description,
  kindLabel,
  ariaLabel,
  density = "editor",
  selected = false,
  width,
  className,
  frameClassName,
  style,
  iconAccessory,
  titleAccessory,
  headerAccessory,
  headerEnd,
  footer,
  body,
  details,
  detailsClassName,
  targetHandle,
  sourceHandle,
  ...articleProps
}: WorkflowNodeCardShellProps) {
  const metadata = getNodeMetadata(kind);
  const Icon = metadata.icon;
  const tokens = DENSITY[density];
  const resolvedWidth =
    width ??
    (density === "editor" || density === "run"
      ? WORKFLOW_NODE_WIDTH
      : undefined);

  return (
    <article
      {...articleProps}
      data-workflow-node-chrome={density}
      className={cn(
        "group/workflow-node border bg-card shadow-sm outline-none transition-[border-color,box-shadow] duration-200",
        tokens.radius,
        selected
          ? "border-foreground/45 shadow-md ring-2 ring-ring/25"
          : "border-border",
        density === "editor" &&
          !selected &&
          "hover:border-foreground/25 hover:shadow-md",
        frameClassName,
        className,
      )}
      style={{
        ...(resolvedWidth !== undefined ? { width: resolvedWidth } : undefined),
        ...style,
      }}
      aria-label={ariaLabel}
    >
      {targetHandle}
      <div className={cn("flex items-start", tokens.gap, tokens.headerPad)}>
        <span
          className={cn(
            "relative flex shrink-0 items-center justify-center",
            tokens.iconBox,
            metadata.tone,
          )}
        >
          <Icon className={tokens.iconSize} stroke={1.9} />
          {iconAccessory}
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-1.5">
            <h4 className={cn("min-w-0 truncate", tokens.title)}>{title}</h4>
            {titleAccessory}
            <span className="rounded bg-muted px-1.5 py-0.5 text-[9px] font-medium text-muted-foreground">
              {kindLabel}
            </span>
            {headerAccessory}
          </div>
          {body ?? <p className={tokens.description}>{description}</p>}
        </div>
        {headerEnd}
      </div>
      {details !== undefined && details !== null && (
        <>
          {density === "editor" && (
            <div className="mx-auto w-4/5 border-t border-border" />
          )}
          <div className={cn("px-3 pb-3 pt-2", detailsClassName)}>
            {details}
          </div>
        </>
      )}
      {footer !== undefined && footer !== null && (
        <div
          className={cn(
            density === "editor"
              ? "flex items-center justify-between px-3 py-2 text-[10px] text-muted-foreground"
              : density === "stage"
                ? "px-6 pb-6 pt-5"
                : "border-t border-border/70 px-3 py-2 text-[10px] text-muted-foreground",
          )}
        >
          {footer}
        </div>
      )}
      {sourceHandle}
    </article>
  );
}
