# ora-plugin-lifecycle

`ora-plugin-lifecycle` owns backend-only orchestration for installed Ora plugins. It joins
filesystem discovery, durable eligibility, process-scoped runtime state, and application
invalidations behind one lifecycle interface.

This crate is the sole owner of plugin processes. Nothing else in Ora starts, stops, or reaps one,
which is what keeps the runtime state reported to the settings surface identical to the processes
that actually exist. Enabling a plugin is therefore also what starts it: durable intent and
reported runtime never disagree beyond the transition itself.

Consumers that need to speak a protocol over a plugin attach to it instead of launching it. An
attachment pairs the running process with the notification stream of that same launch; because one
process emits exactly one stream, the stream is moved to its single consumer, and a plugin whose
stream a previous consumer already claimed is restarted rather than shared. This is how the agent
runtime reaches an agent plugin: it owns the ACP stream of one launch, while enable, stop, scan,
and uninstall keep deciding how long the process lives.

Only explicit scans rebuild installed package identity. List responses resolve the current
Plugin Configuration summary from immutable package declarations and plugin-global value files.
Per-plugin actions operate on cached identity, serialize changes for the same plugin, and allow
unrelated plugins to progress independently.
Missing durable state means disabled, and only the first enable creates a durable row. A package
with an invalid Plugin Configuration declaration remains visible but cannot be enabled. Uninstall
stops the process, then atomically stages the complete
`plugins/installed/<namespace>/<name>` tree and, when selected, `data/<namespace>/<name>` before
deleting durable state. A repository or staging failure rolls the moves back; after commit,
staging cleanup is independent and empty namespace directories are pruned.
Cleanup failures retain their staged paths in memory and are retried by later scans without
reversing the already committed uninstall.
Each scan reapplies durable eligibility to every retained package, stops runtimes that durable state
no longer permits, and removes durable rows for packages missing from disk. To return one coherent
snapshot, a scan acquires cached plugin operation locks in stable identifier order and may therefore
wait for an in-flight launch or stop to finish. This intentionally favors reconciliation consistency
over a partially refreshed result.
Filesystem package parsing remains in `ora-plugin-manager`, process protocol ownership remains in
`ora-plugin-runtime`, and durable eligibility remains behind the `ora-application` repository port.
The production adapter launches Deno through the shared process-tree supervisor with the sandbox
permissions the contribution kind requires, and waits for confirmed process exit before filesystem
cleanup. Startup discovery also reports every package that was skipped, because a package that
never became a plugin is otherwise invisible to an operator.

Transport adapters and concrete dependency composition belong to `ora-backend` and Desktop. This
crate does not depend on Tauri, SQLite, or backend-private state.
