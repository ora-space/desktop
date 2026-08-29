# surface

Frontend side of plugin custom UI ("surfaces"): the header launcher, the
embedded right-panel host, and the glue that mirrors the host's surface
registry into the shell.

## Responsibilities

- `surface-definitions.ts`: flattens installed ui plugins into openable entries.
- `surface-launcher.tsx`: the top-right header entry (button for one surface,
  menu for several, nothing without any). Always a sibling of `DragRegion`.
- `use-open-surface.ts`: opens embedded when the host supports it, otherwise
  windowed; an embedded result claims `sidePanelInstance` in the surface store.
- `surface-host.tsx` + `use-surface-bounds.ts`: the DOM placeholder for an
  embedded surface. Bounds (CSS px + `devicePixelRatio`) are re-measured on
  resize/scroll/transition end, coalesced per animation frame, and never sent
  while hidden; becoming visible sends `setVisible(true)` then one measurement.
- `surface-occlusion.ts`: `useSurfaceVisibility` combines the `@ora/ui`
  occlusion lease count, document visibility, and slot ownership. Overlay
  primitives (`Dialog`, `AlertDialog`, `Sheet`, `Drawer`, `DropdownMenu`,
  `ContextMenu`, `Popover`, and `CommandDialog` through `Dialog`) lease in
  their Root components; `Tooltip` intentionally does not. Window blur never
  hides a surface.
- `surface-event-bridge.tsx`: hydrates the store and forwards lifecycle events.
- `surface-download-toaster.tsx`: download completion/failure toasts. A
  completion carrying an import session is left to the prompt component.
- `surface-download-prompt.tsx`: the user side of webview-plugin downloads. It
  shows queued `downloadChoice` prompts one at a time (import as skill, save
  as, or dismiss/discard), asks the host save dialog for a `save_as`
  destination, and opens the shared skill-import review dialog for resolved and
  automatic `import_skill` sessions.

## Interactions

- `state/stores/surface-store.ts` is the single source of truth for instances
  and the side-slot owner. `WorkspaceReviewLayout` reacts to
  `sidePanelInstance`: non-null opens its `"surface"` panel, null closes it;
  closing the panel in the UI releases the slot and calls `surfaces.close`.
  Claims bump `sidePanelClaimTick`, so re-claiming the instance already in the
  slot wins the panel back even after another panel took it over.
- `app-shell.tsx` places the `Toaster` bottom-left while an embedded surface is
  visible so toasts are not covered by the native view.
