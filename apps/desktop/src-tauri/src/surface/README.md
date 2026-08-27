# surface

Desktop host for plugin UI surfaces. This is the only module that touches Tauri webview APIs on
behalf of `ora-surface`; every lifecycle decision is made by that crate's registry and state
machine, and this module executes the resulting effects.

## Responsibilities

- `service.rs`: `SurfaceService<G, R>`, the composition root. Resolves the surface an installed
  plugin contributes through the plugin gateway, downgrades `embedded`
  to `windowed` when unsupported, drives `SurfaceRegistry` commands, exposes the download
  resolve/discard entry points, and implements the lifecycle's `SurfaceCloser` through
  `SurfaceCloserHandle`.
- `effects.rs`: executes `SurfaceEffect`s (create, destroy, reparent, visibility, emit) outside
  the registry lock, feeds completions back, wires window close requests to `Close`, emits
  `surface://event` to the `main` webview, and arms idle stop.
- `commands.rs`: the `surface_*` Tauri commands and their DTOs; translation only.
- `workbench_assets.rs`: the `ora-plugin://` protocol serving a workbench instance's
  package-shipped files. Resolves the caller label through the registry, requires the URL to
  name that label's instance, resolves the file inside the canonical asset root, applies the
  content-type table and the workbench CSP; every refusal is a 404 with the reason logged.
- `workbench_bridge.rs`, `workbench_api.js`: `plugin_webview_invoke`, the single command a
  `plugin-webview:*` webview may call. Resolves the caller to a live workbench instance, checks
  the method against the effective allowlist (manifest ∩ current registration), bounds the
  payload, starts the plugin on demand, and maps failures onto the `{kind: host | plugin}` error
  union. The instance is pinned to the first process generation it successfully reaches; a call
  that observes a different generation closes the stale instance instead of forwarding.
  `workbench_api.js` is injected into workbench webviews and defines `window.ora.invoke`, whose
  body nests the payload under `request` to match the command signature.
- `spec.rs`, `hooks.rs`: `SurfaceWebviewSpec` (immutable build parameters derived from the
  source: entry URL, navigation policy, web data, injected script), the local `SurfaceBuilder`
  trait implemented for both Tauri builders, `SurfaceHooks::attach` (navigation, popup, download
  hooks) and `apply_spec`. A popup is never given an Ora webview: an allowed remote-site URL is
  handed to the system browser through `PopupOpener` and the request is denied; workbench pages
  never open popups.
- `windowed.rs`: `WindowedAdapter` (stable `WebviewWindowBuilder`).
- `embedded.rs`: `EmbeddedAdapter` (`Window::add_child`), compiled only with the
  `embedded-surfaces` feature.
- `migrate.rs`: popout/dock. With the feature both reparent the webview; without it popout is
  close-and-reopen-windowed and dock is `UNSUPPORTED`.
- `web_data.rs`: `WebDataPolicy` to `ResolvedWebData` (profile directory on Windows/Linux, UUID
  v5 data store identifier on macOS).
- `downloads.rs`: `DownloadDispatcher`, the `DownloadSink` attached to every surface webview:
  selects the disposition from the plugin's manifest rules against the initiating page URL,
  reserves `.part` files in the plugin's `webview/downloads/` directory, promotes or removes
  them, matches browser completions to requests in start order per `(label, url)`, and either
  prompts the trusted main webview (`downloadChoice`) or runs the automatic host action before
  any success is reported.
- `download_actions.rs`: `DownloadActionHost`, the executor of host-owned download actions
  (skill import), implemented by `Backend` and shared by the prompt flow
  (`surface_resolve_download`) and the automatic flow so the two can never drift apart.
- `idle.rs`: per-plugin idle timers; the process is stopped 30 s after the last instance closes
  unless a surface reopens.
- `gateway.rs`: `SurfacePluginGateway` / `SurfaceConnection`, the narrow port onto
  `ora-backend::PluginGateway` that tests replace with a fake.
- `capabilities.rs`: feature flag plus runtime probe (Wayland without `GDK_BACKEND=x11` has no
  child webviews).

## Non-responsibilities

- Lifecycle transitions, singleton policy, label format, navigation policy, asset URL and CSP
  decisions, the managed-download state machine: `ora-surface`.
- Manifest validation: `ora-plugin-manifest` / `ora-plugin-manager`. Plugin processes and data
  directories: `ora-plugin-lifecycle` via the backend gateway.
- The user-facing download choice dialog and skill-import review: the frontend
  (`packages/app-shell`, `SurfaceDownloadPrompt`).

## Invariants

- Labels are resolved through the registry before any asset, bridge call, or download is
  accepted; a page can never choose the destination plugin or path.
- The Tauri command ACL (`capabilities/`, documented in the crate README) grants
  `plugin-webview:*` webviews `plugin_webview_invoke` only and `remote-webview:*` webviews
  nothing; the registry lookup inside the command is the second boundary, not a substitute for
  the first.
- A workbench instance talks to exactly one plugin process generation: the binding is made at
  the first successful bridge connection, later calls compare against it, and a mismatch closes
  the instance. There is no exit-notification channel from the lifecycle to this host, so the
  close happens on the instance's next call rather than eagerly.
- `downloadCompleted` is only emitted after the download's host action actually ran; a failed
  action emits `downloadFailed` and removes the landed file.
- Plugin process failures never fail a surface; only bridge calls degrade.
- No registry lock is held while a webview is created, destroyed, or moved.
- `surface_set_bounds` / `surface_set_visible` only act on embedded instances and are ignored
  otherwise. Bounds are CSS pixels, which Tauri treats as logical units; `scale` is informational.
- `install` registers the `SurfaceCloser` (disable/stop/uninstall close surfaces before the
  process stops), installs the `DownloadActionHost`, and closes every surface when the main
  window is destroyed.
