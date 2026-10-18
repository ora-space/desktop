# Cloud Contract

English | [中文](cloud-contract.zh.md)

In the cloud form the Controller is a gRPC client of Ora Cloud. The only source of the contract is the
Cloud repository's `proto/ora/cloud/internal/v1/` (package `ora.cloud.internal.v1`). Cloud is the
server of every service and owns authoritative persistence; the Controller only dials out and exposes
no service. Tenancy stays in Cloud: the contract carries only opaque identities Cloud already
authorized. Semantics are fixed by the
[Cloud-owned internal gRPC contract](https://github.com/ora-space/specs/blob/main/decisions/cloud/controller-integration/0-cloud-owned-internal-grpc-contract.md)
and [Cloud-owned contract and Controller dial-out](https://github.com/ora-space/specs/blob/main/decisions/controller/api-boundary/20260922-cloud-owned-contract-and-controller-dial-out.md)
ADRs; Cloud's own view is in its `docs/runtime/controller-integration` document. How the Controller
uses each service is in [Controller](controller.md) and [Agent relay](agent-relay.md).

## Services

| Service | Methods | Key points |
|---|---|---|
| `ControllerLeaseService` | `AcquireLease`, `RenewLease`, `ReleaseLease` | One global coordination lease; its `epoch` fences every write |
| `ExecutionService` | `ClaimWork`, `RecordDispatch`, `TakeOverNodeEvent`, `RecordQueriedResult`, `GetDispatch`, `ListPendingDispatches` | Writes carry a `submission_id`: identical content replays the original result, different content fails with `ABORTED` + `CONFLICT`. Dispatch only after `RecordDispatch`, acknowledge only after `TakeOverNodeEvent`; `RecordQueriedResult` never authorizes an acknowledgement |
| `ControlSignalService` | `Watch` (server stream) | `WorkAvailable`, `OperationAvailable`, `Drain`, `NodeAssignment`; at most once, not persisted, changes no ownership |
| `WorkspaceOperationService` | `ClaimOperation`, `PlanEffect`, `RecordEffectResult`, `AdvanceOperation`, `DeferOperation`, `RecordProvisioningFact`, `ListLiveSandboxes` | Writes fenced by `epoch` and the operation `version`; the clone and plugin steps dispatch through `ExecutionService` with the operation's id; `RecordProvisioningFact` is the only entry still open to a creation Cloud abandoned after its blocked deadline, and it receipts one late observation on the residual ledger without reopening anything |
| `NodeReportService` | `RegisterNode`, `ReportNodeStatus`, `ReportNodeIdle`, `EndNode` | Sandbox Nodes the Controller holds sessions with; `RegisterNode` is idempotent per (sandbox, incarnation) and a second live incarnation conflicts until the first ends |
| `AgentRunService` | `TakeOverThreadEvents`, `ClaimThreadCommands`, `RecordThreadCommandDelivered`, `GrantRevisionUpload`, `GrantRevisionDownload` | Thread relay and Revision grants |
| `RuntimeControlService` | `ListBindings`, `AcknowledgeBinding`, `GetExecutionPermit`, `GetEffectPermit`, `ListForceStops`, `ConfirmForceStop` | Fenced runtime control, see [Runtime control](runtime-control.md) |

Failures use the gRPC status code as the primary classification with an attached
`ErrorDetail { ErrorCode }`. The Rust side maps them once, inside the Cloud RPC adapter, to the
coordination classes (conflict, missing, unavailable, unknown, stale eligibility), so coordination
logic never sees gRPC. Cloud answers a stale operation snapshot (`stale_operation`) with the same
`FAILED_PRECONDITION` as a stale lease; the adapter keeps it apart from both the lease verdict and an
ordinary conflict, so only the lease verdict drops the held epoch, and only an effect observation the
closed operation can no longer take is handed over as a provisioning fact.

## Revision upload checksums

`GrantRevisionUploadRequest.checksums` maps each fixed object key to its lowercase SHA-256. Every
returned grant signs `x-amz-checksum-sha256` and `If-None-Match: *`, so an unexpired grant cannot
overwrite an object Cloud already verified; a retry that gets 412 keeps the first object and submits
the original metadata for verification. The uploader sends every returned header with its PUT.
Grants stay in memory and never enter journals, logs, environment, arguments or files, and raw
deployment S3 credentials never reach a Node. Each execution freezes its object bytes and metadata
before the first PUT, so a restart reuses them with fresh grants; changed content needs a new Cloud
work item and keys. Cloud verifies existence, size and stored SHA-256, then atomically records the
Node evidence, receipt and Revision.

Resumed runs extend the same contract: `AgentSessionSpec.prior_revision` names the prior Revision and
its verified bundle object, `DeliverRevisionSpec.prior_revision` only its final commit, and
`GrantRevisionDownload` signs a memory-only `GET` of exactly that bundle for a registered session
without a result.

## Obtaining and generating the contract

`third_party/cloud` is a git submodule of the Cloud repository whose gitlink pins the contract commit.
Its working tree is a partial clone (`--filter=blob:none`) with a sparse checkout limited to `proto/`.

- `task proto:init` (Linux and macOS) clones and checks out the pinned commit, or moves an initialized
  submodule to it. It uses `buf` from `PATH` or `~/.local/bin` and otherwise installs Buf from its
  official GitHub release into `~/.local/bin` without sudo (network required). A plain `git clone`
  does not initialize the submodule. Windows builds use the committed generated client and need
  neither the submodule nor `buf`.
- `crates/controller-proto` (`ora-controller-proto`) holds only the generated code under `src/gen/`,
  produced by `buf` with pinned remote plugins (`neoeinstein-prost`, `neoeinstein-tonic`); no local
  `protoc` is needed. Server stubs are generated behind the `test-server` feature: production never
  compiles them, and the Controller's tests use them to host an in-memory Cloud over the real contract.
- `task proto:generate` regenerates. `task proto:check` (part of `task lint:crates`) verifies the
  submodule is at its pinned commit with no local changes under `proto/`, regenerates and fails on any
  diff. `buf lint` and `buf breaking` belong to the Cloud repository.

## Upgrading the contract

1. Cloud merges the contract change (`v1` accepts additive changes only; a breaking change is a
   sibling `v2`).
2. `git -C third_party/cloud fetch origin <commit> && git -C third_party/cloud checkout <commit>`;
   the commit must be on Cloud's main or a protected branch.
3. Run `task proto:generate`, adapt the adapter, and commit the gitlink, the generated code and the
   change together, linking the Cloud change from the PR.

Generation proves structural agreement only; behavioral agreement with the real Cloud server is
exercised by the cluster repository's Compose stack (Gateway → Cloud → Controller → Sandbox Server →
Node).
