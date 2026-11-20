# Surfaces

English | [中文](surfaces.zh.md)

A surface is the UI a `workbench` or `webview` plugin contributes, shown in an isolated native
webview, either docked into the main window (`embedded`) or as its own window (`windowed`). A
workbench plugin ships a page inside its package, served by the host and connected to the plugin
process through one request bridge; a webview plugin embeds an external HTTPS site and has no
process. Host internals are in
[`apps/desktop/src-tauri/src/surface/README.md`](../../apps/desktop/src-tauri/src/surface/README.md).

| Layer | Crate or module | Owns |
|---|---|---|
| Manifest | `ora-plugin-manifest`, `ora-plugin-manager` | `[workbench]`/`[webview]` sections: entry page, bridge methods, allowed origins, download policy |
| Domain | `ora-surface` | Ids and labels, `SurfaceDefinition`, `NavigationPolicy`, asset URLs and CSP, the instance state machine, `SurfaceRegistry`, downloads, events |
| Process | `ora-plugin-lifecycle` via `PluginGateway` | Plugin data directories, `ensure_running`, generation-pinned connections, `SurfaceCloser` |
| Host | `apps/desktop/src-tauri/src/surface/` | Tauri webviews, commands, events, downloads and host actions, idle stop, `ora-plugin://` assets, the bridge |
| Frontend | `packages/app-shell` (`SurfaceCapability`) | Placeholder layout, bounds reporting, download prompts and toasts, the Skill import review |

The host never decides lifecycle on its own: every command goes through `SurfaceRegistry`, which
returns `SurfaceEffect`s that the host executes. The registry is also the authorization source: a
webview label is trusted only after `resolve_label` maps it to a record.

## Isolation

| Label | May invoke |
|---|---|
| `main` | Every command, including `surface_*` |
| `plugin-webview:*` | Only `plugin_webview_invoke` |
| `remote-webview:*` | Nothing; the page has no Tauri IPC |

The ACL is the outer boundary; identity is still resolved inside the command, so a workbench page
that reached `plugin_webview_invoke` could only address its own instance.

`window.open` from a surface never creates an Ora webview: a remote URL inside the allow list goes to
the system browser and every popup request is denied. Workbench pages never open popups.

Each plugin has one persistent web profile under its data directory (`web-profile/`), so a webview
plugin keeps its login and a workbench page keeps its own `localStorage` even where all `ora-plugin://` pages share one origin.
The profile is never exposed to any page or to `ora/storage/*`.

## Opening and closing

`surface_open` resolves the surface from the installed manifest, registers an `Opening` instance,
creates the webview, completes the registry with `Opened` and emits `opened`. Opening does not start
the plugin process: a workbench page starts it with its first bridge call. Webview surfaces are
singletons (a second open focuses the existing window); workbench surfaces may have several
instances.

Windowed instances turn `CloseRequested` into a registry close; closing is never blocked. Destroying
the main window closes every surface. The lifecycle calls `SurfaceCloser` before stopping or
uninstalling a plugin, so its surfaces close first. When a plugin's last instance closes, a 30-second
idle timer is armed and the process is stopped if no instance reopened.

## Commands and events

| Command | Purpose |
|---|---|
| `surface_capabilities` | `{ embedded }` |
| `surface_list` | Current `SurfaceRecord`s |
| `surface_open` | Open `{ pluginId, target }`; `embedded` degrades to `windowed` when unsupported |
| `surface_close`, `surface_reload` | Close; reload the page (a `failed` instance is rebuilt) |
| `surface_set_bounds`, `surface_set_visible` | Embedded placement in CSS px (Tauri logical units) |
| `surface_popout`, `surface_dock` | Move between windowed and embedded |
| `surface_resolve_download`, `surface_discard_download` | Answer a prompted download |

`SurfaceRecord` is `{ instance, pluginId, kind, title, target, state }` with `state` in `opening`,
`open`, `migrating`, `closing` or `failed`. Errors use the shared contract: `plugin_not_found`,
`resource_in_use`, `invalid_request`, `internal_error`. `surface://event` carries `SurfaceEvent`
(`opened`, `migrated`, `migrateFailed`, `failed`, `closed`, `downloadStarted`, `downloadChoice`,
`downloadCompleted`, `downloadFailed`); its TypeScript counterpart in
`packages/app-shell/src/platform/types.ts` must match the Rust serde shape.

The `embedded-surfaces` Cargo feature (`tauri/unstable`, off by default) compiles child webviews and
reparenting. `surface_capabilities.embedded` combines that flag with a runtime probe (Linux Wayland
sessions without `GDK_BACKEND=x11` report `false`).

## Workbench surfaces

A workbench plugin declares its entry page and the bridge methods the page may call; both are frozen
into the `SurfaceDefinition` when an instance opens.

**Assets.** Pages load `ora-plugin://localhost/<instance>/<entry>` (on Windows
`http://ora-plugin.localhost/...`). The protocol handler resolves the caller's label through the
registry (the label, not the URL, is the authority), requires the URL's instance segment to match,
the path to be a `PortableRelativePath` inside the asset root, a regular file and a known content
type. Every refusal is a bare 404 with the reason logged. Documents are `Cache-Control: no-store`,
other assets `no-cache`, and documents carry a CSP that forbids inline script and style, remote
content and frames.

**Bridge.** The host injects `window.ora.invoke(method, params)`, which calls
`plugin_webview_invoke`. The host resolves the caller to a live instance, checks the method against
the manifest allowlist, bounds request and response at 1 MiB, starts the plugin if needed (15 s),
intersects the allowlist with what the running generation registered, and invokes the method with a
host-owned envelope:

```jsonc
{ "surface": { "instance_id": 7, "generation": 3 }, "input": /* page params */ }
```

The page cannot name a plugin, instance or generation. Rejections are either
`{ kind: "host", code }` (`SURFACE_UNAVAILABLE`, `PAYLOAD_TOO_LARGE`, `METHOD_NOT_ALLOWED`,
`METHOD_UNAVAILABLE`, `PLUGIN_UNAVAILABLE`, `PLUGIN_CALL_TIMED_OUT`, `INTERNAL`) or
`{ kind: "plugin", code, message }`, with the plugin's message stripped of control characters and
capped at 1 KiB, so a plugin cannot impersonate host conditions.

**Generation binding.** An instance is pinned to the first plugin generation its bridge reaches. A
later call that sees another generation (the process crashed or restarted) closes the stale instance
and fails with `SURFACE_UNAVAILABLE`; reopening yields a fresh instance. Without an exit
notification from the lifecycle, this happens on the instance's next call.

## Webview surfaces

A webview plugin names a start URL and an exact set of allowed HTTPS origins; navigation outside
them is denied and logged.

### Downloads

The manifest maps page-URL rules to a disposition: `{ reject }`, `{ auto = "import_skill" }` or
`{ prompt = [actions...] }` with actions `import_skill` and `save_as` (`save_as` needs a destination
and is refused in `auto`). A download is attributed solely by the webview label, its disposition is
chosen from the initiating page URL, and the file lands as a unique `.part` in
`data/<namespace>/<name>/webview/downloads/` before being promoted on success or removed on failure.

- **Prompt.** The landed file waits in `AwaitingChoice` and `downloadChoice` goes to the main
  webview, which shows queued prompts one at a time and answers with `surface_resolve_download`
  (running the chosen action) or `surface_discard_download` (deleting the file). One download can
  never run two actions.
- **Auto.** The host runs the action itself. `downloadCompleted` is emitted only after it succeeds;
  for `import_skill` it carries `importSessionId` and the frontend opens the Skill import review, see
  [Agent definitions and Skills](../agents/agents-and-skills.md#importing-skills). A failed action
  emits `downloadFailed` and removes the file.
