# Agent stop ownership

English | [中文](session-stop.zh.md)

Agent launches have separate synced attempt locks under `session-attempts`. The plugin and every
host-spawned child inherit the attempt descriptor. Synced PID/start-time/boot clues also protect
against a child closing inherited descriptors. On session shutdown and MCP process replacement, Node closes spawn admission
and immediately kills the processes the attempt mark claims (below) and the original process
groups identified by the durable clues.
The plugin can die before it receives ora/shutdown. Node awaits lifecycle cleanup for diagnostics
and verifies stop evidence before releasing resources. A closed spawn gate disables automatic restarts.
Signals, timeout and a free lock alone are not stop proof. An unresolved attempt retains package
leases, workload views and checkout responsibility across restart. Terminal events and revision
admission wait for confirmation and resource release.

Every process the spawn gate starts also carries `ORA_SESSION_ATTEMPT=<attempt>`, which its
descendants inherit. Before the ledger records the attempt, `session-attempts/<attempt>.mark`
durably records the attempt's start, in boot clock ticks, and the user its processes run as. This
reaches tool jobs that leave every original group and drop the inherited lock, such as a `cmd &`
in an Agent tool shell started under `setsid` (OpenCode's bash tool, Claude's Bash), adopted by
init, from a runtime like Bun that closes inherited descriptors. A live process belongs to the
attempt when it carries the mark and started at or after the attempt, or when its parent belongs
to it. Termination freezes every such process with SIGSTOP through a pidfd, rescans until no new
one appears (at most 32 passes), then kills them all, before it signals the original groups. Stop
proof also requires two consecutive clean scans: a claimed live process keeps the attempt
running; an unreadable scope or procfs, or an unreadable process of the attempt's user while the
Node runs as another user without `CAP_SYS_PTRACE`, leaves the evidence unknown. A confirmed
attempt with a mark scope records the proof `lock_processes_and_marks`; an attempt an older Node
registered has no scope, none of its processes carries a mark, and it records
`lock_and_processes` as before. Neither same-boot proof counts for `after-host-reboot`
reconciliation. Schema 15 admits the new value; older programs refuse a schema 15 database.

**What dies, and when.** When a session ends (EndSession, an Agent failure, a cancel that ends the
session, graceful shutdown, or restart recovery after a Node crash) and when MCP lifecycle replaces
the session's Agent process, every live process carrying that attempt's mark is killed before the
stop is confirmed. That includes everything the Agent started: tool background jobs, dev servers,
watchers, MCP and language servers, and shared daemons such as a Gradle daemon or an sccache
server that a tool in the session launched. Nothing is killed between turns. A user who needs a
daemon to outlive the session must start it outside Ora; one started by the Agent dies with the
session even when other sessions or programs use it.

**Known limits.** The mark is Linux only and does not make containment strong.
- A process that clears its environment (`env -i`) or makes itself non-dumpable is found only while
  its parent belongs to the attempt; once orphaned it is invisible. In particular, an orphaned
  process of the attempt's own user whose environment the Node cannot read is not counted, and does
  not make the evidence unknown: unrelated programs (ssh-agent, binaries with file capabilities)
  are unreadable in the same way, and counting them would block every stop.
- A Node that itself runs under another attempt's mark (a nested Node started by an outer Agent)
  passes both on as `outer:attempt`, so the outer attempt's stop also kills the inner Node's
  attempts. The inner Node only ever claims processes listing its own attempt.
- The mark is not a secret: a process that can read it can start processes carrying it, and those
  die with the attempt, but it cannot make the Node signal a process that started before the
  attempt.

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
this execution's Agent process. Node records the stop request and kills the old attempt's marked processes and
original process groups before waiting for the old supervisor to retire. Its cleanup completes before a successor
is installed. The latest target is prepared before replacement; affirmative
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
`stop_unconfirmed`. Automatic waiting then stops, retaining the slot and leases. Live completion,
automatic recovery and explicit root reconciliation wake retained owners after durable stop proof. Root diagnostics on
`session-stop.sock` accept the newline-delimited request `{"operation":"probe_capacity"}` and
return total slots, running probes, cleanup_pending, queued intents, available slots and named residues (execution, attempt, state and original bound). Session
queries expose only anonymous availability and their own members.

Upgrade deployment requirement: Node home and package/workload views must be on supported local
filesystems. Homes on NFS, 9p or virtiofs are refused by the new stop-evidence checks. Drain sessions
and move the deployment's owned data to a supported local filesystem before this upgrade; copying
unconfirmed responsibilities to another host does not establish stop proof.

A probe owner task panic fails the Node closed. Its slot, exact-version leases and durable responsibility remain retained until restart recovery proves cleanup; continuing with a lost owner is not supported. Stop-request and kill_all results are diagnostic only: proof comes from the original lock plus process clues and the attempt mark scan, or the configured trusted reboot evidence.
