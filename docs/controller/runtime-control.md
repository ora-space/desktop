# Runtime Control

English | [中文](runtime-control.zh.md)

In Cloud mode, who may act on a runtime Workspace is decided by Cloud; the Controller only delivers
what Cloud authorized, and the Node isolates stale authority at its real entry points. The governing
decision is the
[fenced runtime control delivery](https://github.com/ora-space/specs/blob/main/decisions/cloud/controller-integration/20260927-fenced-runtime-control-delivery.md)
ADR; tenant role policy and exclusive control are Cloud's (see Cloud's `docs/runtime/runtime-control`
document) and are not duplicated by the Controller or the Node.

## Identities and generations

Each identity answers a different question and none stands in for another:

| Identity | Authority |
|---|---|
| User and membership | Cloud authentication and authorization |
| Controller lease epoch | The current coordinator's right to write |
| Workspace runtime generation and sandbox id | One running instance and its durable target |
| Runtime control epoch and session | One user or maintenance qualification |
| Node and host incarnation | One running program instance |
| Execution and effect id | Durable responsibility for fixed input |

A stable execution id is not a new authorization and cannot be reused with an old permit to start a
second task. Changing a generation only refuses new actions; it never proves an old process stopped.

## Authenticated management channels

The Cloud–Controller and Controller–Node management links authenticate mutual TLS service identity
and target binding; declared ids in messages are only consistency checks. The Controller refuses a
cloud configuration without `management_tls`, a non-HTTPS Cloud endpoint, non-HTTPS effects or a
non-WSS router, see [Controller](controller.md#cloud-form). The Controller only dials out and keeps no
Cloud business state in SQLite.

In the Node image, root-owned management processes (the Node and the process host) hold the
configuration, journals and private keys under the root-only `/run/ora`, while Git workloads and
agent plugins run as UID/GID 1000 and cannot read management material or write control journals, see
[Container images](../operations/container-images.md). Static Git configuration may carry only
non-secret CA trust, never personal credential helpers or an SSH fallback.

## Dispatch, closure and reporting

1. Cloud checks permission, control, the running instance and conflicting activity in one
   transaction and stores the fixed execution input.
2. Before every new dispatch the Controller obtains a fresh permit (`GetExecutionPermit`, or
   `GetEffectPermit` for Substrate effects). If Cloud is unavailable or the permit is invalid, no new
   action starts.
3. The Node persists acceptance before executing. Retransmitting the same identity with the same input
   only returns the existing state. Bare, unfenced commands are accepted only on private local IPC in
   a Node home that never enabled runtime control; once a home has enabled it, that fallback is closed
   for good.

Withdrawing a qualification closes new admission: the Controller delivers the input-closed binding to
the Node (`ListBindings`, `AcknowledgeBinding`), unaccepted old actions are refused and accepted work
finishes within its original scope. The Node reports the closure and its unfinished execution ids, and
Cloud hands control over only after reconciling them. Closure is sticky within an epoch. A lost
response stays unknown and never releases anything from a cache; a user losing permission does not
discard the end facts of an execution already running.

The Node persists its runtime target, control epoch and version, closure state and responsibilities.
A lower epoch, a different binding at the same epoch, a wrong incarnation or a wrong target is
refused, both at the acceptance queue and at the first real mutation, and a restart never reopens an
old entry from memory.

## Non-acceptance proof for expired deliveries

A Cloud business settlement is not a Node terminal. The Controller requests original dispatch
fences only after opting in through Hello and negotiating `execution_non_acceptance`. Older
Controllers receive no unfamiliar capability; an older Node keeps unknown responsibility without
raw-command fallback or inferring non-acceptance from Unknown.

The Node checks the complete identity ledger and writes an irreversible execution-intake fence in
one SQLite writer transaction. Old permits, a new control epoch and restart cannot bypass it; its
peer delivery family remains independent. Accepted/completed work produces no proof. An empty new
incarnation cannot prove absence in the old instance, while a persisted original proof can replay.

`RecordExecutionNonAcceptance` takes over a separate proof without a normal event ACK. Only commit
success clears the corresponding unresolved observation; explicit committed proof identities
recover lost responses. Global quiesce still closes dispatch and checks clones, plugins, sessions,
Revision delivery, artifact uploads and all other unknown responsibilities.

## Force stop

Emergency termination uses an independent effect permit lasting at most 10 seconds and never beyond
the Controller's lease. The Controller's force-stop worker (`ListForceStops`, `ConfirmForceStop`)
keeps delivering while an ordinary lifecycle step waits on its Node, and never invents execution
completion. The execution side uses a durable runtime fence so a late ensure cannot recreate the
target; Cloud accepts a termination only when it proves both `terminated` and `lateEnsureFenced`.
Unreachability, a heartbeat timeout or a new generation never permits parallel writes to the same
directory.

The Controller does not filter force-stop plans by the requester's role: the same RPC delivers plans
from different Cloud entry points. Every effect and sandbox ID is preserved, including targets whose
ensure never executed. A target rejection, execution failure or unreachable Substrate does not starve
independent targets. Cloud unavailability during permit or confirmation calls, or local lease loss,
stops new dispatch. Every new PUT checks permit epoch, expiry and the current local lease after the
journal GET, and checks again after any bounded Node control close; preparation never extends a permit.
Local dispatch requires more than 2 seconds of permit lifetime remaining, matching Sandbox Server.
For lifecycle effects, permit expiry within the same epoch enters RetryWait; the next attempt fetches
a fresh permit and continues the original effect. An epoch mismatch ends the round without writing
defer state.

Before a new terminate PUT, the worker attempts to remove the existing Fleet target and send its
Shutdown control close. A missing connection or close acknowledgement does not prevent termination.
This is neither an EndSession nor termination evidence; EndSession still comes only from the existing
authorized Cloud command queue. GET404, an unreachable Node, an expired permit, and missing or false
dual-evidence fields cannot confirm termination. The exact `sandbox_terminated` ensure refusal (a 503
admission error or a failed journal's top-level error) enters the existing Blocked path; recovery GET
cannot retransmit that ensure. Ordinary 503, Docker failures and stale permission are not reclassified
as tombstones, and successful journals retain their historical results.

The `ora-controller` tests in `src/cloud/substrate/put_admission.rs`,
`tests/workspaces/force_stop.rs`, `tests/workspaces/permit_expiry.rs` and
`src/cloud/operations/eligibility.rs` exercise permit, control-close, termination-evidence and tombstone
refusal boundaries over real HTTP/gRPC/WebSocket protocols with local doubles. They do not prove
production authorization, real container destruction, or joint Cloud–Controller–Substrate fault
recovery. The Controller never invents targets Cloud omitted.

## Compatibility

Peers negotiate `runtime_control` explicitly, and a controlled execution never silently falls back to
an unbound request. Retired listeners and the old JSON surface stay closed while historical
responsibilities remain queryable. Upgrades keep historical owners, executions and audit references;
rollback never works by deleting new durable state. Node certificates are short-lived and have no
seamless refresh, so rotation needs a managed restart. Old workload-owned management homes are not
repaired automatically: unfinished unbound history blocks new binding, and deployments must stop and
reconcile old processes before a controlled migration.
