# Agent stop ownership

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
