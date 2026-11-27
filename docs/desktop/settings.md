# Settings

English | [中文](settings.zh.md)

Settings are user preferences stored in the SQLite `user_config` key/value table. `ora-db` only reads
and writes raw values; typed interpretation, defaults and validation belong to the application
layer (`crates/application/src/user_config.rs`) and, for the worktree root, to the Backend. The
Backend `Settings` interface schedules persistence on the blocking executor, and a malformed stored
value fails explicitly instead of being coerced. The frontend keeps no second copy of any
preference.

| Key | Meaning | Operations |
|---|---|---|
| `developer_mode` | Reveals developer-only controls | `getDeveloperMode`, `setDeveloperMode` |
| `log_level` | Process-wide runtime log level | `getRuntimeLogLevel`, `setRuntimeLogLevel` |
| `network_proxy_mode` | System, Manual or Direct | `getProxySettings`, `setProxySettings`, `clearProxySettings` |
| `network_proxy_settings` | Optional host-level HTTP proxy | `getProxySettings`, `setProxySettings`, `clearProxySettings`, `checkProxySettings` |
| `network_proxy_check_url` | Last explicitly confirmed HTTP(S) probe URL | `getProxyCheckUrl`, `setProxyCheckUrl` |
| `worktree_root` | Creation root for new task worktrees | Native `get_worktree_root`, `set_worktree_root` |

## Developer mode and developer options

The Developer options category is always visible so the switch can always be reached. When the
initial read fails the switch stays disabled, developer-only controls stay hidden and the page
offers retry; a failed update keeps the last backend response.

With developer mode on, the page reveals the runtime log-level selector and **Download logs**, which
exports today's log through the native save dialog on hosts that expose that capability. Turning it
off hides and unmounts both without navigating away. Plugin log levels and plugin log downloads
appear in the plugin management menu under the same condition, see
[Plugin logging](../plugins/plugin-logging.md).

Developer mode controls discoverability only. It grants no permission and changes no transport
authorization.

## Log level

A change applies to the current process immediately and is persisted in `user_config.log_level`.
The selector shows the authoritative effective level, and Trace and Debug carry a high-volume
warning. Startup restores the persisted level and defaults to `info` only when unset; environment
variables have no effect. Update ordering and rollback are described in
[Logging](../core/logging.md#desktop-configuration).

## Network proxy

Desktop defaults to **Follow system proxy**. **Manual proxy** also accepts a host, port and optional
HTTP Basic credentials. **Direct connection** ignores proxy settings and environment variables.
An existing manual configuration remains selected on upgrade. Changing mode retains the manual
endpoint, and an older Desktop version continues to use that endpoint after downgrade. Clear
explicitly removes the saved endpoint and restores the default mode.

On Windows, System reads the current user's static WinHTTP configuration. When automatic
detection is the only setting, Ora connects directly and skips WPAD. An explicit PAC URL without
a static proxy produces an unsupported-configuration error. On Linux, System reads GNOME static
settings through GIO when a persistent backend is available and otherwise honors `HTTP_PROXY`,
`HTTPS_PROXY`, `ALL_PROXY` and `NO_PROXY`. Windows integrated proxy authentication is outside
this version's support. Loopback and system bypass targets go directly.
Connection or proxy resolution failures are reported without retrying the request directly.

These rules apply to Desktop updates and marketplace sources that opt in
([Marketplace](../plugins/marketplace.md)). The connection test follows redirects using the
selected mode and resolves the route for each destination. A final HTTP 2xx response succeeds
and shows the final route. Other HTTP responses show the status, and HTTP 407 reports a proxy
authentication failure.
Enter a bare hostname, `http://hostname`, or `https://hostname` for a manual proxy. The port
field supplies the port, and `https://` uses TLS to the proxy itself.
Checking uses the current form and Desktop updates use
saved settings. In manual mode, a final direct response cannot confirm the proxy.

## Cloud connection

The server address (a normalized origin) is stored as `user_config.cloud_server`; the sign-in token is
not here but in the operating system's secure store. Address rules, sign-in and the token are
described in [Desktop host](desktop-host.md#cloud-connection).

## Worktree root

The worktree root lives in Settings → Data & privacy. It must be an absolute path to an existing
directory and defaults to `~/.ora/worktrees`. It is a creation target only, see
[Desktop host](desktop-host.md#persistent-paths).

## Marketplace sources

Marketplace sources are managed in Settings → Plugins → Manage marketplace. The dialog's behavior
and the source fields are described in [Marketplace](../plugins/marketplace.md#sources).

## Upgrades and read failures

When the initial proxy read fails, Settings shows an error and retry action instead of an empty editor that could overwrite the saved value. A background read failure keeps the current draft visible and disables save, clear and connection checks until retry succeeds.

Preference migration retains legacy agent selections. A legacy package name is resolved to its full namespaced identity only when exactly one installed agent matches; ambiguous or absent selections remain unchanged.

Only a newly created database requests default marketplace source seeding. Existing empty lists, including those created by older releases without a deletion marker, survive upgrades and restarts. First-install initialization also respects sources supplied by a distributor.

Fresh-install provenance is committed with migration bookkeeping before schema steps. Interrupted
migrations or distribution initialization resume their pending work on restart; source seeding
does not depend on the distribution's optional first-install SQL.

Connection checks default to `https://google.com`, with an editable URL. A failed check describes
reachability of that URL, which can depend on the site or network; it does not invalidate or change
saved proxy settings. Background read failures keep an open check dialog and its URL draft while
blocking checks until recovery.

Confirming a check saves its HTTP(S) URL separately in `user_config` before probing, even if the
target is unreachable. Cancelling discards the current draft and reopening restores the saved URL;
restarts and upgrades retain it. Only a missing preference uses the Google default. Read failures
remain errors, and a failed URL save keeps the draft and skips the probe. **Check and save**
appears for unsaved proxy settings and saves the tested settings after a successful check.
If the check fails, bypasses a manual proxy, or the settings cannot be saved, the draft stays
available. **Check** tests settings that are already saved.
