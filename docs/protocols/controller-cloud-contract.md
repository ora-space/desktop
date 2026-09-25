# Controller–Cloud Contract

English | [中文](controller-cloud-contract.zh.md)

In a cloud deployment the Controller calls Cloud (Ora Cloud, the Go API Server) over gRPC: it claims
work, registers before dispatching, takes over Node events, stores queried results, reads for
recovery, and opens one server stream on which Cloud sends signals. The single source of the
contract is the **Cloud repository**'s `proto/ora/cloud/internal/v1/` (package
`ora.cloud.internal.v1`); Cloud is the server of every service and owns authoritative persistence,
the Controller only dials out and exposes no gRPC service to Cloud. This repository never copies
the `.proto` files; it only holds tonic code generated from a pinned commit, of which production uses
only the **client**. Tenancy
stays in Cloud: the contract carries only opaque identities Cloud has already authorized, with no
tenant, user or membership fields.

The semantics belong to specs
`decisions/cloud/controller-integration/0-cloud-owned-internal-grpc-contract.md`; how this
repository consumes them is fixed by
`decisions/controller/api-boundary/20260922-cloud-owned-contract-and-controller-dial-out.md`.
This page only covers how the contract is obtained, generated and upgraded here. The runtime
integration is the `CloudStore` adapter described in the
[Controller runtime](../controller/local-runtime.md): lease, `ExecutionService` and the `Watch` stream
are consumed, the stream deciding when to claim as fixed by
`decisions/controller/api-boundary/20260924-controller-consumes-watch-signals.md`. Its persistence semantics follow
`decisions/controller/persistence/20260922-coordination-store-with-sqlite-and-cloud-adapters.md`.

## Services and key semantics

| Service                  | Methods                                                                                                           | Key points                                                                                                                                                                                                                                                                                                       |
| ------------------------ | ----------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `ControllerLeaseService` | `AcquireLease` / `RenewLease` / `ReleaseLease`                                                                    | Global coordination lease; `epoch` fences every write                                                                                                                                                                                                                                                            |
| `ExecutionService`       | `ClaimWork`, `RecordDispatch`, `TakeOverNodeEvent`, `RecordQueriedResult`, `GetDispatch`, `ListPendingDispatches` | Writes carry `submission_id`: same identity with identical content returns the original result, different content fails with `ABORTED`+`CONFLICT`; dispatch only after `RecordDispatch` succeeds, acknowledge only after `TakeOverNodeEvent` succeeds, `RecordQueriedResult` never authorizes an acknowledgement |
| `ControlSignalService`   | `Watch` (server stream)                                                                                           | `WorkAvailable` / `Drain` / `NodeAssignment`; at-most-once, not persisted, changes no ownership; after a stream loss fall back to periodic `ClaimWork`                                                                                                                                                           |

Failures use the gRPC status code as the primary classification with `ErrorDetail{ErrorCode}`
attached; the Rust side maps them once, inside the Cloud RPC adapter, to the persistence
coordination classes (conflict, missing, unavailable, unknown, stale eligibility), so coordination
logic never sees gRPC.

## Obtaining the contract: submodule + sparse-checkout

`third_party/cloud` is a git submodule of the Cloud repository whose gitlink pins the contract
commit; the working tree is a partial clone (`--filter=blob:none`) with a sparse-checkout limited
to `proto/`.

- `task proto:init` (Linux / macOS; the generated client is committed, so Windows builds need neither the submodule nor buf): a first run clones with `--no-checkout --filter=blob:none --sparse`, runs
  `sparse-checkout set proto`, then `git submodule update --init` to the pinned commit; an already
  initialized submodule is only moved to the pinned commit. The crates CI job runs the same task.
  It reuses `buf` from `PATH` or `~/.local/bin`; if absent, it uses `curl` to install Buf 1.73.0
  from the [official GitHub release](https://buf.build/docs/cli/installation/) into `~/.local/bin`
  without sudo. Downloads require network access. Protocol tasks include this directory in their
  `PATH`; add it to your shell's `PATH` if you want to invoke `buf` directly.
- A plain `git clone` or `actions/checkout` does not initialize it; dependency initialization is an
  explicit action.
- `/specs` remains an ignored, independent checkout and is not a contract dependency.

## Generation and checks

`crates/controller-proto` (`ora-controller-proto`) holds only the generated output under `src/gen/`,
produced by `buf` with pinned remote plugins (`neoeinstein-prost`, `neoeinstein-tonic`) from
`third_party/cloud/proto`; this needs network access, not a local `protoc`. The server modules are
generated behind `#[cfg(feature = "test-server")]`: production never compiles them, and the
Controller's tests enable the feature to host an in-memory Cloud over the real contract.

- `task proto:generate`: regenerate.
- `task proto:check`: verify the submodule is at its pinned commit with no local changes under
  `proto/`, then regenerate and fail on any diff; part of `task lint:crates`.
- `buf lint` and `buf breaking` belong to the Cloud repository and are not repeated here.

## Upgrading the contract

1. Cloud merges the contract change (its `v1` accepts additive changes only; a breaking change is a
   sibling `v2`).
2. `git -C third_party/cloud fetch origin <commit> && git -C third_party/cloud checkout <commit>`;
   the referenced commit must be on Cloud's main or a protected branch and stay fetchable.
3. `task proto:generate`, adapt the adapter, and commit the gitlink, the generated code and the
   change together; link the Cloud change from the PR so reviewers can expand the submodule diff.

Generation proves structural agreement. The Controller's tests exercise the adapter against an
in-memory Cloud built on the generated server stubs; behavioral agreement with the real Cloud gRPC
server is verified end to end through the minicloud cloud form and is not yet an automated test.
