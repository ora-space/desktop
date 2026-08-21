# Plugin Surfaces

A surface is a piece of UI contributed by a plugin of kind `ui`, shown inside an isolated native
webview either docked into the main window (`embedded`) or as its own window (`windowed`). Two
content sources exist: a remote web site (`remote_site`) and a page shipped inside the plugin
package (`panel`), which the host serves itself and connects to the plugin process through a
request/push bridge. Surfaces are declared in the plugin's `orax.toml` under `[[ui.surfaces]]`
(see `crates/plugin-manager/README.md` for the manifest and `specs/plugin/6-ui-webview.md` for
the normative description). Plugins are identified by `ora_domain::PluginId`, spelled
`<namespace>/<name>` on every wire contract (`pluginId`).

## Architecture

| Layer    | Crate / module                                          | Owns                                                                                                                                                                        |
| -------- | ------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Manifest | `ora-plugin-manager` (`surface.rs`, `ui_validation.rs`) | `InstalledSurface`, allow lists, `WebDataPolicy`, `InstancePolicy`, `PanelSource` (canonical asset root + entry)                                                            |
| Domain   | `ora-surface`                                           | ids and labels, `SurfaceDefinition`, `NavigationPolicy`, panel URLs/CSP/content types, the instance state machine, `SurfaceRegistry`, `DownloadCoordinator`, `SurfaceEvent` |
| Process  | `ora-plugin-lifecycle` via `ora-backend::PluginGateway` | plugin data directories, `ensure_running`, `PluginConnection`, notification broadcast, `SurfaceCloser`                                                                      |
| Host     | `apps/desktop/src-tauri/src/surface/`                   | Tauri webviews, commands, events, download delivery, idle stop, `ora-plugin://` assets, the panel bridge and push router                                                    |
| Frontend | `packages/app-shell` (`SurfaceCapability`)              | placeholder layout, bounds reporting, toasts                                                                                                                                |

The host never decides lifecycle on its own: every command goes through `SurfaceRegistry`, which
returns `SurfaceEffect`s, and the host executes them (`effects.rs`). The registry is also the
authorization source: a webview label is only trusted after `resolve_label` maps it to a record.

## Opening a surface

`surface_open` resolves the surface from the installed manifest (refusing disabled plugins),
registers an `Opening` instance, creates the webview synchronously, completes the registry with
`Opened`, emits `opened`, and then links the plugin process:

- process running: `ora/ui/surface_opened { surface_id, surface_instance_id, plugin_generation }`
  is sent directly;
- process stopped or failed: it is started in a spawned task (`ensure_running`, 15 s) and, once
  running, receives `ora/ui/surface_opened` for every open instance of the plugin;
- process starting: the new instance is announced once the start completes. Each instance is
  announced at most once per process generation, even when several opens race one start.

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
| `surface_reload`       | `{ instance }`                             | — (reloads the page; a `failed` instance is rebuilt and emits `opened` again)       |

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

A `window.open` from a surface page never creates an Ora webview: a remote-site URL inside the
allow list is handed to the system browser (`PopupOpener`) and every popup request is denied, so
no page can obtain a window that escapes the navigation policy or the registry.

Windowed instances listen for `CloseRequested` and turn it into a registry `Close`; the close is
never blocked. Destroying the main window closes every surface. The lifecycle calls the
registered `SurfaceCloser` before stopping, disabling, or uninstalling a plugin, so its surfaces
are closed first.

## Plugin data directory and downloads

```
<data-dir>/plugins/installed/<name>/                         read-only package (orax.toml, main.js, panel assets)
<data-dir>/plugins/data/<namespace>/<name>/
  downloads/                 host-written surface downloads
  web-profile/<surface_id>/  persistent web profile (Windows / Linux)
```

The data directory is keyed by plugin identity (`<namespace>/<name>`), so it survives reinstalls;
panel assets are read from the installed package directory selected at discovery. The plugin process has no
filesystem permissions and no environment variable naming the directory: it reads `downloads/`
and its own files back through the `ora/storage/*` host methods (see the `ora-plugin-lifecycle`
README), addressed by the same logical paths the host uses in its notifications. `web-profile/`
is never exposed to the plugin.

A download is attributed solely by the webview label: `resolve_label` yields the plugin, the
file is reserved as `<downloads>/<name>.part`, promoted to its unique final name on success
(`ora-utils::fs` sanitization and collision handling) or removed on failure. The frontend gets
`downloadStarted` / `downloadCompleted` / `downloadFailed`; for windowed surfaces the main window
is brought forward on completion. Delivery to the plugin runs in a spawned task under a
semaphore of 8: `ensure_running` (15 s) followed by `ora/ui/download_completed` with

```jsonc
{ "surface_id", "surface_instance_id", "plugin_generation",
  "download": { "id", "page_url", "source_url", "file_name",
                "path" /* logical: "downloads/<file_name>" */, "size_bytes",
                "completed_at" /* local RFC 3339 */ } }
```

Plugin errors are logged, never retried; the file stays on disk.

## Panels

A `panel` surface declares `[ui.surfaces.source]` with `kind = "panel"`, `root = "<dir>"`, and
`entry = "<file>.html"`. Both paths are validated at discovery: `root` must be a subdirectory of
the installed package directory and `entry` an existing `.html` file below it. The host never
serves anything outside the canonical `root`.

### Assets: `ora-plugin://`

Panel webviews load `ora-plugin://localhost/<namespace>/<name>/<surface_id>/<entry>` (on
Windows `http://ora-plugin.localhost/...`). The protocol handler (`panel_assets.rs`) resolves the caller
webview label through `SurfaceRegistry` — the label, not the URL, is the authorization source —
and then requires the URL's plugin and surface segments to match that record, the remaining path
to be a `PortableRelativePath` resolving inside the asset root (`CanonicalPathRoot`), a regular
file, and an extension in the content-type table (`html js mjs css json svg png jpg jpeg webp woff
woff2 wasm`, plus `map` in debug builds). Every refusal is a bare 404; the reason is logged.
Documents are served with `Cache-Control: no-store`, every other asset with `no-cache` (asset
URLs are not versioned, so a package update must not be masked by a cached script), and
documents carry this CSP:

```
default-src 'none'; base-uri 'none'; object-src 'none'; frame-src 'none'; frame-ancestors 'none';
form-action 'none'; worker-src 'none'; connect-src ipc: http://ipc.localhost;
script-src <base>; style-src <base>; img-src <base> data:; font-src <base>
```

Inline script and style are therefore impossible; a panel page ships external JS/CSS. Panels
always get a web profile of their own (`web-profile/<surface_id>` under the plugin data directory,
the persistent mechanism), may only navigate below their own asset base, and never open
popups. An incognito store is deliberately not used: on Linux custom URI schemes are bound to the
web context and an incognito webview's fresh ephemeral context never receives `ora-plugin://`.

### Bridge: page ⇄ host ⇄ plugin

The host injects `panel_api.js` into every panel webview:

```ts
window.acquireOraSurfaceApi(): {
  version: 1;
  request(payload: JsonValue): Promise<JsonValue>;           // rejects with SurfaceError
  onPush(listener: (e: { sequence: number; payload: JsonValue }) => void): () => void;
}
type SurfaceError =
  | { kind: "host"; code: "SURFACE_CLOSED" | "PAYLOAD_TOO_LARGE" | "PLUGIN_UNAVAILABLE" | "TIMEOUT" | "INTERNAL" }
  | { kind: "plugin"; code: number; message: string };
```

`request` invokes the `surface_request` command, the bridge command used by `panel-surface:*`
webviews. The host resolves the caller
label to a live panel instance, bounds the payload at 1 MiB, starts the plugin if needed
(`ensure_running`, 15 s), and invokes

```jsonc
// host → plugin request
"ora/ui/request" { "surface_id", "surface_instance_id", "plugin_generation", "payload" }  →  { "payload" }
```

A `PluginMethodError` from the plugin arrives as `{ kind: "plugin", code, message }` with the
message stripped of control characters and capped at 1 KiB; host conditions use the `host` kind
so a plugin cannot impersonate them.

Plugins push with the notification

```jsonc
// plugin → host notification (declared in `emits`)
"ora/ui/push" { "surface_id", "surface_instance_id", "plugin_generation", "payload" }
```

The backend's `BroadcastNotificationSink` fans every plugin notification out;
`SurfaceService::route_pushes` delivers `ora/ui/push` by calling `window.__ORA_SURFACE_PUSH__` in
the owning webview after checking that the instance is a live panel of that plugin and surface,
that the `plugin_generation` in the params is the generation of the emitting process, and that
this generation is the one the host currently talks to (a restarted process's predecessor is
dropped). Envelopes carry a per-instance `sequence` starting at 1; pushes that
arrive before the page registered a listener are buffered (64) and replayed. Delivery is
best-effort: a lagging router or a reload loses pushes, and a page that needs consistency re-reads
its state through `request`.

A panel plugin must register `ora/ui/request` and declare `ora/ui/push` in `emits` (checked at
the handshake like `ora/ui/download_completed` for remote sites). All `ora/ui/*` names live in
`ora-plugin-lifecycle::registration`; the desktop host imports them. Plugins written with
`@ora-space/plugin-sdk` use `defineUiPlugin`, which registers the contract and maps the
snake_case params to camelCase.

## Web data isolation

`web_data.mode` of a remote-site surface (default `persistent`):

| Mode         | Windows / Linux                      | macOS                                                                               | Other                        |
| ------------ | ------------------------------------ | ----------------------------------------------------------------------------------- | ---------------------------- |
| `persistent` | `web-profile/<surface_id>` directory | data store identifier = UUID v5 (URL namespace, `ora://surface/<plugin>/<surface>`) | shared default store, warned |
| `ephemeral`  | incognito                            | incognito                                                                           | incognito                    |

## Idle stop

When a plugin's last instance closes, a 30 s timer is armed; reopening a surface cancels it. On
expiry the instance count is re-checked before `stop_if_idle` is called.

## Embedded surfaces feature

`embedded-surfaces = ["tauri/unstable"]` in `apps/desktop/src-tauri/Cargo.toml` compiles the
child-webview adapter and reparenting. It is off by default; `surface_capabilities.embedded` is
the compile flag combined with a runtime probe (Linux Wayland sessions without
`GDK_BACKEND=x11` report `false`).
