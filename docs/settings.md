# Settings

Ora Desktop presents Settings through the App Shell and the Tauri-backed contracts client.

## Developer mode

Settings always includes a Developer options category, whose page contains the developer-mode switch. Its authoritative value is `user_config.developer_mode`, interpreted by the application layer over the generic SQLite key/value adapter; the frontend does not persist a second copy. A failed initial read leaves the switch disabled, keeps developer-only controls hidden on that page, and offers retry. A failed update retains the last backend response.

Developer mode controls discoverability only. It does not grant permissions, change transport authorization, or make backend operations inaccessible when disabled.

## Developer options

The Developer options navigation category remains available regardless of the developer-mode value so users can always reach its switch. When developer mode is enabled, the same page reveals the process-wide log-level selector; disabling it hides and unmounts that selector without navigating away.

Log-level changes take effect for the current Desktop process and are persisted in `user_config.log_level`. The selector displays the authoritative effective level, including an active startup override, without naming `ORA_LOG_LEVEL` or exposing startup-source details. Trace and Debug include a high-volume warning.

See [Runtime Logging](runtime-logging.md) for startup precedence and rollback behavior.
