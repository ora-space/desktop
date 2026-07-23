import type { ReactNode } from "react";
import { cn } from "@ora/ui";

/**
 * Turns the flexible span of a frameless-window title strip into an OS drag
 * handle.
 *
 * On the desktop shell Tauri reads `data-tauri-drag-region` off the element the
 * pointer lands on, so the children render `pointer-events-none`: a press
 * anywhere over the strip resolves to this element instead of an inner text node
 * that carries no attribute. Keep interactive controls (buttons, badges) as
 * siblings *outside* this element - anything drawn inside it is unclickable, and
 * anything drawn beside it never starts a window drag. The attribute is inert in
 * the browser, so the same header ships to Web unchanged.
 */
export function DragRegion({
  className,
  children,
}: {
  className?: string;
  children?: ReactNode;
}) {
  return (
    <div
      data-tauri-drag-region=""
      className={cn("flex min-w-0 flex-1 items-center self-stretch", className)}
    >
      {children !== undefined && (
        <div className="pointer-events-none flex min-w-0 items-center gap-2">{children}</div>
      )}
    </div>
  );
}
