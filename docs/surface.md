# Plugin Surfaces

A surface is a piece of UI contributed by a plugin of kind `ui`. The first supported source is
a remote web site (`remoteSite`) shown inside an isolated native webview, either docked into the
main window (`embedded`) or as its own window (`windowed`).

## Architecture

| Layer    | Crate / module                                          | Owns                                                                                                                                          |
| -------- | ------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------- |
| Manifest | `ora-plugin-manager` (`surface.rs`, `ui_validation.rs`) | `InstalledSurface`, allow lists, `WebDataPolicy`, `InstancePolicy`                                                                            |
| Domain   | `ora-surface`                                           | ids and labels, `SurfaceDefinition`, `NavigationPolicy`, the instance state machine, `SurfaceRegistry`, `DownloadCoordinator`, `SurfaceEvent` |
| Process  | `ora-plugin-lifecycle` via `ora-backend::PluginGateway` | plugin data directories, `ensure_running`, `PluginConnection`, `SurfaceCloser`                                                                |
| Host     | `apps/desktop/src-tauri/src/surface/`                   | Tauri webviews, commands, events, download delivery, idle stop                                                                                |
| Frontend | `packages/app-shell` (`SurfaceCapability`)              | placeholder layout, bounds reporting, toasts                                                                                                  |

The host never decides lifecycle on its own: every command goes through `SurfaceRegistry`, which
returns `SurfaceEffect`s, and the host executes them (`effects.rs`). The registry is also the
authorization source: a webview label is only trusted after `resolve_label` maps it to a record.

## Opening a surface

`surface_open` resolves the surface from the installed manifest (refusing disabled plugins),
registers an `Opening` instance, creates the webview synchronously, completes the registry with
`Opened`, emits `opened`, and then links the plugin process:

- process running: `ui/surfaceOpened { surfaceId, instanceId, generation }` is sent directly;
- process stopped or failed: it is started in a spawned task (`ensure_running`, 15 s) and, once
  running, receives `ui/surfaceOpened` for every open instance of the plugin;
- process starting: the new instance is announced once the start completes.

A plugin that fails to start does not fail the surface; remote sites do not depend on it.
Singleton surfaces that are already open return the existing record and focus its window.

## Commands (main webview only)

| Command                | Request                                    | Response                                                                            |
| ---------------------- | ------------------------------------------ | ----------------------------------------------------------------------------------- |
| `surface_capabilities` | —                                          | `{ embedded, webDataIsolation }`                                                    |
| `surface_list`         | —                                          | `SurfaceRecord[]`                                                                   |
| `surface_open`         | `{ pluginId, surfaceId, target }`          | `SurfaceRecord` (actual target; `embedded` degrades to `windowed` when unsupported) |
| `surface_close`        | `{ instance }`                             | —                                                                                   |
| `surface_set_bounds`   | `{ instance, x, y, width, height, scale }` | — (embedded only; CSS px = Tauri logical units)                                     |
| `surface_set_visible`  | `{ instance, visible }`                    | — (embedded only)                                                                   |
| `surface_popout`       | `{ instance }`                             | — (reparent with `embedded-surfaces`; otherwise close + reopen windowed)            |
| `surface_dock`         | `{ instance }`                             | — (`embedded-surfaces` only; otherwise `invalid_request`)                           |
| `surface_reload`       | `{ instance }`                             | —                                                                                   |

`SurfaceRecord` is `{ instance, pluginId, surfaceId, title, target, state }` with `state` in
`opening | open | migrating | closing | failed`. Errors use the shared command error contract:
`plugin_not_found`, `plugin_disabled`, `resource_in_use` (busy instance), `invalid_request`
(unknown instance or surface, unsupported operation), `internal_error`.

## Events

`surface://event` carries `ora_surface::SurfaceEvent` serialized with a camelCase `type` tag:
`opened`, `migrated`, `migrateFailed`, `failed`, `closed`, `downloadStarted`,
`downloadCompleted`, `downloadFailed`. The TypeScript `SurfaceEvent` in
`packages/app-shell/src/platform/types.ts` is the contract the Rust serde attributes must match.

## Windows and closing

Windowed instances listen for `CloseRequested` and turn it into a registry `Close`; the close is
never blocked. Destroying the main window closes every surface. The lifecycle calls the
registered `SurfaceCloser` before stopping, disabling, or uninstalling a plugin, so its surfaces
are closed first.

## Plugin data directory and downloads

```
<data-dir>/plugin-data/<plugin_id>/
  downloads/                 host-written surface downloads
  web-profile/<surface_id>/  persistent web profile (Windows / Linux)
```

A download is attributed solely by the webview label: `resolve_label` yields the plugin, the
file is reserved as `<downloads>/<name>.part`, promoted to its unique final name on success
(`ora-utils::fs` sanitization and collision handling) or removed on failure. The frontend gets
`downloadStarted` / `downloadCompleted` / `downloadFailed`; for windowed surfaces the main window
is brought forward on completion. Delivery to the plugin runs in a spawned task under a
semaphore of 8: `ensure_running` (15 s) followed by `ui/downloadCompleted` with

```jsonc
{ "surfaceId", "instanceId", "generation",
  "download": { "id", "pageUrl", "sourceUrl", "fileName", "path", "sizeBytes",
                "completedAt" /* local RFC 3339 */ } }
```

Plugin errors are logged, never retried; the file stays on disk.

## Web data isolation

| Policy              | Windows / Linux                      | macOS                                                                               | Other                        |
| ------------------- | ------------------------------------ | ----------------------------------------------------------------------------------- | ---------------------------- |
| `persistentProfile` | `web-profile/<surface_id>` directory | data store identifier = UUID v5 (URL namespace, `ora://surface/<plugin>/<surface>`) | shared default store, warned |
| `ephemeralIsolated` | incognito                            | incognito                                                                           | incognito                    |

## Idle stop

When a plugin's last instance closes, a 30 s timer is armed; reopening a surface cancels it. On
expiry the instance count is re-checked before `stop_if_idle` is called.

## Embedded surfaces feature

`embedded-surfaces = ["tauri/unstable"]` in `apps/desktop/src-tauri/Cargo.toml` compiles the
child-webview adapter and reparenting. It is off by default; `surface_capabilities.embedded` is
the compile flag combined with a runtime probe (Linux Wayland sessions without
`GDK_BACKEND=x11` report `false`).
