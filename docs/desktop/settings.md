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
| `network_proxy_settings` | Optional host-level HTTP proxy | `getProxySettings`, `setProxySettings`, `clearProxySettings`, `checkProxySettings` |
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

The proxy is a host, port and optional HTTP Basic credentials, stored as one JSON value. It is used
by marketplace traffic that opts in (Git sync and artifact downloads per source, see
[Marketplace](../plugins/marketplace.md)) and by the Desktop updater. `checkProxySettings` probes an
absolute HTTP(S) URL through the supplied settings, independent of what is saved: any HTTP response,
including 4xx and 5xx, means the proxy reached a host, while timeouts, refused connections and
invalid URLs report `unreachable` rather than failing the command.

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
