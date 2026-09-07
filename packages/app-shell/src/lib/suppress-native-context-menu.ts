import { useEffect } from "react";

/**
 * Suppresses the browser/OS right-click context menu (e.g. Refresh, Back,
 * Inspect Element) across the whole window. Surfaces that provide their own
 * context menu (Base UI ContextMenu triggers) call stopPropagation before this
 * document-level listener runs, so those custom menus keep working while every
 * area without a right-click listener shows nothing.
 */
export function useSuppressNativeContextMenu(): void {
  useEffect(() => {
    function onContextMenu(event: MouseEvent): void {
      event.preventDefault();
    }
    document.addEventListener("contextmenu", onContextMenu);
    return () => document.removeEventListener("contextmenu", onContextMenu);
  }, []);
}
