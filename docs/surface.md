# Plugin Surfaces

A surface is the piece of UI a `workbench` or `webview` plugin contributes, shown inside an
isolated native webview either docked into the main window (`embedded`) or as its own window
(`windowed`). The two plugin kinds are the two content sources: a workbench plugin ships a page
inside its package, which the host serves itself and connects to the plugin process through a
single request bridge; a webview plugin embeds an external HTTPS site and has no process of its
own. Plugins are identified by `ora_domain::PluginId`, spelled `<namespace>/<name>` on every
wire contract (`pluginId`).

## Architecture

| Layer    | Crate / module                                          | Owns                                                                                                                                                              |
| -------- | ------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Manifest | `ora-plugin-manifest`, `ora-plugin-manager`             | the `[workbench]` / `[webview]` manifest sections: entry page, declared bridge methods, allowed origins, download policy                                          |
| Domain   | `ora-surface`                                           | ids and labels, `SurfaceDefinition`, `NavigationPolicy`, workbench asset URLs/CSP/content types, the instance state machine, `SurfaceRegistry`, downloads, events |
| Process  | `ora-plugin-lifecycle` via `ora-backend::PluginGateway` | plugin data directories, `ensure_running`, generation-pinned connections, `SurfaceCloser`                                                                         |
| Host     | `apps/desktop/src-tauri/src/surface/`                   | Tauri webviews, commands, events, the download pipeline and host actions, idle stop, `ora-plugin://` assets, the workbench bridge                                 |
| Frontend | `packages/app-shell` (`SurfaceCapability`)              | placeholder layout, bounds reporting, download prompts and toasts, the skill-import review                                                                        |

The host never decides lifecycle on its own: every command goes through `SurfaceRegistry`, which
returns `SurfaceEffect`s, and the host executes them (`effects.rs`). The registry is also the
authorization source: a webview label is only trusted after `resolve_label` maps it to a record.

## Command ACL

Which Tauri commands a webview may invoke is decided by its label through the capabilities in
`apps/desktop/src-tauri/capabilities/` (see that crate's README for the file layout):

| Label              | May invoke                                                                        |
| ------------------ | --------------------------------------------------------------------------------- |
| `main`             | every Ora command, including all `surface_*` commands                             |
| `plugin-webview:*` | `plugin_webview_invoke` only; the page reaches its plugin through that one bridge |
| `remote-webview:*` | nothing: no capability targets the label, so the page has no Tauri IPC at all     |

The ACL is the outer boundary; identity is still resolved inside the command.
`plugin_webview_invoke` takes the caller from the webview label and rejects a label the registry
does not know, so a workbench page that somehow reached the command could still only address its
own instance.

## Opening a surface

`surface_open` resolves the surface from the installed manifest (refusing disabled plugins),
registers an `Opening` instance, creates the webview synchronously, completes the registry with
`Opened`, and emits `opened`. The plugin process is not started by opening: a workbench page
starts it on demand with its first bridge call, and a webview plugin has no process at all.

Webview surfaces are singletons: a second open returns the existing record and focuses its
window. Workbench surfaces may open multiple instances.

## Commands (main webview only)

| Command                    | Request                                    | Response                                                                            |
| -------------------------- | ------------------------------------------ | ----------------------------------------------------------------------------------- |
| `surface_capabilities`     | —                                          | `{ embedded }`                                                                      |
| `surface_list`             | —                                          | `SurfaceRecord[]`                                                                   |
| `surface_open`             | `{ pluginId, target }`                     | `SurfaceRecord` (actual target; `embedded` degrades to `windowed` when unsupported) |
| `surface_close`            | `{ instance }`                             | —                                                                                   |
| `surface_set_bounds`       | `{ instance, x, y, width, height, scale }` | — (embedded only; CSS px = Tauri logical units)                                     |
| `surface_set_visible`      | `{ instance, visible }`                    | — (embedded only)                                                                   |
| `surface_popout`           | `{ instance }`                             | — (reparent with `embedded-surfaces`; otherwise close + reopen windowed)            |
| `surface_dock`             | `{ instance }`                             | — (`embedded-surfaces` only; otherwise `invalid_request`)                           |
| `surface_reload`           | `{ instance }`                             | — (reloads the page; a `failed` instance is rebuilt and emits `opened` again)       |
| `surface_resolve_download` | `{ downloadId, action, destination? }`     | `{ action, importSessionId }` (runs one host action for a prompted download)        |
| `surface_discard_download` | `{ downloadId }`                           | — (dismisses a prompted download and deletes the landed file)                       |

`SurfaceRecord` is `{ instance, pluginId, kind, title, target, state }` with `kind` in
`workbench | webview` and `state` in `opening | open | migrating | closing | failed`. Errors use
the shared command error contract: `plugin_not_found`, `plugin_disabled`, `resource_in_use`
(busy instance), `invalid_request` (unknown instance, unsupported operation, unknown download or
action), `internal_error`.

## Events

`surface://event` carries `ora_surface::SurfaceEvent` serialized with a camelCase `type` tag:
`opened`, `migrated`, `migrateFailed`, `failed`, `closed`, `downloadStarted`, `downloadChoice`,
`downloadCompleted`, `downloadFailed`. The TypeScript `SurfaceEvent` in
`packages/app-shell/src/platform/types.ts` is the contract the Rust serde attributes must match.

## Windows and closing

A `window.open` from a surface page never creates an Ora webview: a remote-site URL inside the
allow list is handed to the system browser (`PopupOpener`) and every popup request is denied, so
no page can obtain a window that escapes the navigation policy or the registry. Workbench pages
never open popups at all.

Windowed instances listen for `CloseRequested` and turn it into a registry `Close`; the close is
never blocked. Destroying the main window closes every surface. The lifecycle calls the
registered `SurfaceCloser` before stopping, disabling, or uninstalling a plugin, so its surfaces
are closed first. When a plugin's last instance closes, a 30 s idle timer is armed; on expiry
the instance count is re-checked before the process is stopped.

## Workbench surfaces

A workbench plugin declares its page entry and the bridge methods the page may call in the
manifest; both are frozen into the `SurfaceDefinition` when an instance opens.

### Assets: `ora-plugin://`

Workbench webviews load `ora-plugin://localhost/<instance>/<entry>` (on Windows
`http://ora-plugin.localhost/...`). The protocol handler (`workbench_assets.rs`) resolves the
caller webview label through `SurfaceRegistry` — the label, not the URL, is the authorization
source — and then requires the URL's instance segment to match that record, the remaining path
to be a `PortableRelativePath` resolving inside the asset root (`CanonicalPathRoot`), a regular
file, and an extension in the content-type table. Every refusal is a bare 404; the reason is
logged. Documents are served with `Cache-Control: no-store`, every other asset with `no-cache`,
and documents carry a CSP that forbids inline script and style, remote content, and frames; a
workbench page ships external JS/CSS from its own package.

### Bridge: page → host → plugin

The host injects `workbench_api.js` into every workbench webview:

```ts
window.ora.invoke(method: string, params?: JsonValue): Promise<JsonValue>;
// rejects with:
type BridgeError =
  | { kind: "host"; code: "SURFACE_UNAVAILABLE" | "PAYLOAD_TOO_LARGE" | "METHOD_NOT_ALLOWED"
        | "METHOD_UNAVAILABLE" | "PLUGIN_UNAVAILABLE" | "PLUGIN_CALL_TIMED_OUT" | "INTERNAL" }
  | { kind: "plugin"; code: number; message: string };
```

`invoke` calls the `plugin_webview_invoke` command with the payload nested under `request`, the
only command the `plugin-webviews` capability allows (see [Command ACL](#command-acl)). The host
resolves the caller label to a live workbench instance, checks the method against the manifest
allowlist, bounds request and response at 1 MiB, starts the plugin if needed (`ensure_running`,
15 s), intersects the allowlist with the methods the running generation registered, and invokes
the method with a host-owned envelope:

```jsonc
// host → plugin request params
{ "surface": { "instance_id": 7, "generation": 3 }, "input": /* page params */ }
```

The request carries no plugin, instance, or generation field: identity comes exclusively from
the webview label. A `PluginMethodError` from the plugin arrives as
`{ kind: "plugin", code, message }` with the message stripped of control characters and capped
at 1 KiB; host conditions use the `host` kind so a plugin cannot impersonate them.

### Process generation binding

A workbench instance is pinned to the first plugin process generation its bridge successfully
reaches (`SurfaceRegistry::bind_workbench_generation`, first writer wins). A later call that
observes a different generation — the process crashed or was restarted — closes the stale
instance and fails with `SURFACE_UNAVAILABLE` instead of letting page state from one generation
talk to another; reopening the surface yields a fresh instance. There is no exit-notification
channel from the lifecycle to the surface host, so the close happens on the instance's next
call rather than eagerly.

## Webview surfaces

A webview plugin names a start URL and an exact set of allowed HTTPS origins; navigation outside
them is denied and logged. Each plugin gets one persistent web profile under its data directory
(`web-profile/`; on macOS a derived data-store identifier), so login state survives restarts and
is never shared between plugins.

### Downloads

The manifest maps page-URL rules to a disposition: `{ reject }`, `{ auto = "import_skill" }`, or
`{ prompt = [actions...] }` with actions from `import_skill | save_as`. `save_as` requires a
user-chosen destination and is therefore refused in `auto` at manifest validation.

```
<data-dir>/plugins/data/<namespace>/<name>/
  webview/downloads/   host-written landed artifacts
  web-profile/         persistent web profile (never exposed to any page)
```

A download is attributed solely by the webview label. The disposition is selected against the
initiating page URL frozen at request time, the file is reserved as a unique `.part` in the
plugin's `webview/downloads/` directory (`ora-utils::fs` sanitization and collision handling),
and promoted to its final name on success or removed on failure. Browser completions carry no
native download id, so they are matched to requests in start order per `(label, url)`; the same
URL may be downloading concurrently.

- **Prompt**: the landed download parks in `AwaitingChoice` and `downloadChoice` (origin, file
  name, size, allowed actions) goes to the main webview. The frontend's `SurfaceDownloadPrompt`
  shows queued prompts one at a time and answers with `surface_resolve_download` (running the
  chosen action; `save_as` passes the destination from the host save dialog) or
  `surface_discard_download` (deleting the file). Choosing is linearized in the
  `ManagedDownload` state machine, so one download can never run two actions.
- **Auto**: the landed download goes straight to `Processing` and the host runs the action
  itself through the same `DownloadActionHost` the prompt flow uses. `downloadCompleted` is
  emitted only after the action succeeded — for `import_skill` it carries `importSessionId`, and
  the frontend opens the skill-import review on it — while a failed action emits
  `downloadFailed` and removes the landed file.

## Web data isolation

| Kind      | Windows / Linux          | macOS                                                      |
| --------- | ------------------------ | ---------------------------------------------------------- |
| workbench | `web-profile/` directory | data store identifier derived from the plugin id (UUID v5) |
| webview   | `web-profile/` directory | data store identifier derived from the plugin id (UUID v5) |

Both kinds get one persistent profile per plugin: a webview plugin keeps its login state, and a
workbench page keeps its `localStorage` separate from other plugins on platforms where all
`ora-plugin://` pages share one origin.

## Embedded surfaces feature

`embedded-surfaces = ["tauri/unstable"]` in `apps/desktop/src-tauri/Cargo.toml` compiles the
child-webview adapter and reparenting. It is off by default; `surface_capabilities.embedded` is
the compile flag combined with a runtime probe (Linux Wayland sessions without
`GDK_BACKEND=x11` report `false`).
