# Agent stop ownership

English | [中文](session-stop.zh.md)

Agent launches have separate synced attempt locks under `session-attempts`. The plugin and every
host-spawned child inherit the attempt descriptor. Synced PID/start-time/boot clues also protect
against a child closing inherited descriptors. Shutdown closes spawn admission before waiting.
Signals, timeout and a free lock alone are not stop proof. An unresolved attempt retains package
leases, workload views and checkout responsibility across restart. Terminal events and revision
admission wait for confirmation and resource release.

`node.session_stop.confirmation_timeout_ms` defaults to 600000 and must be positive. Its first
deadline is durable; retries do not extend it. Different-boot proof requires an explicit
`trusted_host_id` matching the original host, or `single_mount_with_storage_fencing: true` backed
by deployment fencing. A container machine-id is never inferred as trusted identity. Schema 14
inventories old running Agent executions before migration and retains them as `legacy_unknown`.

Root can request evidence rechecks from the running Node:

```sh
ora-node reconcile-session-stop /absolute/node/home EXECUTION ATTEMPT REQUEST_ID maintenance recheck
```

Reasons are `maintenance`, `probe-capacity`, or `legacy-upgrade`; modes are `recheck` and
`after-host-reboot`. The private socket authenticates the kernel peer UID. The same request ID and
input returns the stored result; conflicting reuse is rejected. The request, original identity,
observation and result are audited without commands, Settings or credentials. Confirmation does
not itself report resources as released. Interrupted audits obtain fresh evidence. Confirmed,
released details retire after seven days without outbox or incomplete-audit references; summaries
and audit records remain. Unconfirmed details never age out.

Node home and separate-workload package views require supported local filesystems on the same
device. Startup checks hard-link support; view creation rejects cross-device copies. Exact MCP
version leases allow another validated MCP version to coexist, but protect leased bytes from
repair or removal. Other plugin kinds retain ID-wide exclusion. Kind changes cannot bypass live
or restart-retained leases.

Lifecycle sessions retain their provider session ID and last active MCP revision after failed
loads or detach. They never rebuild a provider session or replay a transcript to recover an MCP
failure. The next turn can retry once per generation; there is no periodic refresh retry.
An MCP package-version change, or the ninth runtime load after the initial setup, replaces only
this execution's Agent process. The latest target is prepared before replacement; affirmative
stop proof precedes a new attempt. HOME, checkout, model grant and ora-git context remain attached
to the execution. New targets coalesce while cleanup is pending.

Host health is independent of ACP convergence. Resolution queues one automatic probe; manual
attempt numbers coalesce per member. The Node schedules sessions round-robin with eight global
slots and at most two per session, including retained and recovered cleanup. A permit starts a
ten-second budget: up to eight seconds for initialize/tools-list and the remainder for cleanup.
Stdio uses the session's exact package view, checkout and workload identity with only explicit
settings and non-authoritative process necessities (PATH, LANG, LC_ALL, TMPDIR). HTTP uses the Node
identity with proxies and redirects disabled. Healthy requires both
the handshake and confirmed cleanup. Old generation results cannot update current health; unknown
cleanup keeps its permit, lease and durable stop responsibility. No automatic failure retry or TTL
turns an observation into readiness.

Probe cleanup exceeding ten seconds reports `mcp_probe_timeout` once. Its separate, original
stop-confirmation bound still applies; only at that bound does responsibility become
`stop_unconfirmed`. Automatic waiting then stops, retaining the slot and leases. Explicit root
reconciliation wakes retained owners after recording stop proof. Root diagnostics on
`session-stop.sock` accept the newline-delimited request `{"operation":"probe_capacity"}` and
return total slots, running probes, cleanup_pending, queued intents and available slots. Session
queries expose only anonymous availability and their own members.

Upgrade deployment requirement: Node home and package/workload views must be on supported local
filesystems. Homes on NFS, 9p or virtiofs are refused by the new stop-evidence checks. Drain sessions
and move the deployment's owned data to a supported local filesystem before this upgrade; copying
unconfirmed responsibilities to another host does not establish stop proof.
