# surface

Desktop host for plugin UI surfaces. This is the only module that touches Tauri webview APIs on
behalf of `ora-surface`; every lifecycle decision is made by that crate's registry and state
machine, and this module executes the resulting effects.

## Responsibilities

- `service.rs`: `SurfaceService<G, R, C>`, the composition root. Resolves the manifest surface
  through the plugin gateway, refuses disabled plugins, downgrades `embedded` to `windowed` when
  unsupported, drives `SurfaceRegistry` commands, and implements the lifecycle's `SurfaceCloser`
  through `SurfaceCloserHandle`.
- `effects.rs`: executes `SurfaceEffect`s (create, destroy, reparent, visibility, emit) outside
  the registry lock, feeds completions back, wires window close requests to `Close`, emits
  `surface://event` to the `main` webview, announces sessions to the plugin, and arms idle stop.
- `commands.rs`: the `surface_*` Tauri commands and their DTOs; translation only.
- `spec.rs`, `hooks.rs`: `SurfaceWebviewSpec` (immutable build parameters), the local
  `SurfaceBuilder` trait implemented for both Tauri builders, `SurfaceHooks::attach`
  (navigation, popup, download hooks) and `apply_web_data`.
- `windowed.rs`: `WindowedAdapter` (stable `WebviewWindowBuilder`).
- `embedded.rs`: `EmbeddedAdapter` (`Window::add_child`), compiled only with the
  `embedded-surfaces` feature.
- `migrate.rs`: popout/dock. With the feature both reparent the webview; without it popout is
  close-and-reopen-windowed and dock is `UNSUPPORTED`.
- `web_data.rs`: `WebDataPolicy` to `ResolvedWebData` (profile directory on Windows/Linux, UUID
  v5 data store identifier on macOS, incognito for ephemeral surfaces).
- `downloads.rs`: `DownloadDispatcher`, the `DownloadSink` attached to every surface webview:
  reserves `.part` files in the plugin's `downloads/` directory, promotes or removes them,
  notifies the frontend, brings the main window forward for windowed surfaces, and delivers
  `ui/downloadCompleted` to the plugin process under a concurrency limit of 8.
- `plugin_link.rs`: `ui/surfaceOpened` / `ui/surfaceClosed` notifications and the on-demand
  process start with replay of all open instances.
- `idle.rs`: per-plugin idle timers; the process is stopped 30 s after the last instance closes
  unless a surface reopens.
- `gateway.rs`: `SurfacePluginGateway` / `SurfaceConnection`, the narrow port onto
  `ora-backend::PluginGateway` that tests replace with a fake.
- `capabilities.rs`: feature flag plus runtime probe (Wayland without `GDK_BACKEND=x11` has no
  child webviews).

## Non-responsibilities

- Lifecycle transitions, singleton policy, label format, navigation policy, download naming:
  `ora-surface`.
- Manifest validation: `ora-plugin-manager`. Plugin processes and data directories:
  `ora-plugin-lifecycle` via the backend gateway.

## Invariants

- Labels are resolved through the registry before any download is accepted; a remote page can
  never choose the destination plugin or path.
- Plugin process failures never fail a surface; only download delivery degrades.
- No registry lock is held while a webview is created, destroyed, or moved.
- `surface_set_bounds` / `surface_set_visible` only act on embedded instances and are ignored
  otherwise. Bounds are CSS pixels, which Tauri treats as logical units; `scale` is informational.
- Only the `main` webview may invoke `surface_*` commands (`app_commands.rs`,
  `permissions/app-commands.toml`, `task lint:acl`, `tests/command_acl.rs`).
- `install` registers the `SurfaceCloser` so disable/stop/uninstall close surfaces before the
  process stops, and closes every surface when the main window is destroyed.
