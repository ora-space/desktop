# ora-plugin-lifecycle

`ora-plugin-lifecycle` owns backend-only orchestration for installed Ora plugins. It joins
filesystem discovery, durable eligibility, process-scoped runtime state, and application
invalidations behind one lifecycle interface.

Only explicit scans rebuild the installed snapshot. Per-plugin actions operate on cached identity,
serialize changes for the same plugin, and allow unrelated plugins to progress independently.
Filesystem package parsing remains in `ora-plugin-manager`, process protocol ownership remains in
`ora-plugin-runtime`, and durable eligibility remains behind the `ora-application` repository port.
The production adapter launches Deno through the shared process-tree supervisor and waits for
confirmed process exit before filesystem cleanup.

Transport adapters and concrete dependency composition belong to `ora-backend` and Desktop. This
crate does not depend on Tauri, SQLite, or backend-private state.
