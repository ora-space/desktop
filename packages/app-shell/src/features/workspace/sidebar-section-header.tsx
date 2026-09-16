import type { ReactNode } from "react";

/**
 * Quiet group label under an expanded project. Not a TreeRow: clicking it
 * must not toggle the project or steal selection from a child.
 */
export function SidebarSectionHeader({
  label,
  depth = 1,
  action,
}: {
  label: string;
  depth?: 1 | 2;
  action?: ReactNode;
}) {
  return (
    <div
      className="flex h-7 items-center gap-1 pr-1 text-[11px] font-medium text-muted-foreground"
      style={{ paddingLeft: `${8 + depth * 18}px` }}
    >
      <h3 className="min-w-0 flex-1 truncate text-[11px] font-medium">
        {label}
      </h3>
      {action}
    </div>
  );
}

/** Empty-group copy aligned with the rows that would sit under the header. */
export function SidebarSectionEmpty({
  children,
  depth = 1,
}: {
  children: ReactNode;
  depth?: 1 | 2;
}) {
  return (
    <p
      className="py-0.5 pr-2 text-[11px] text-muted-foreground/80"
      style={{ paddingLeft: `${8 + (depth + 1) * 18}px` }}
    >
      {children}
    </p>
  );
}
