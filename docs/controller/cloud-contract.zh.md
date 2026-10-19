# Cloud 契约

[English](cloud-contract.md) | 中文

在 cloud 形态下，Controller 是 Ora Cloud 的 gRPC 客户端。契约唯一的来源是 Cloud 仓库的 `proto/ora/cloud/internal/v1/`（包 `ora.cloud.internal.v1`）。Cloud 是所有服务的服务端，拥有权威的持久化；Controller 只拨出，不暴露任何服务。租户信息留在 Cloud：契约只携带 Cloud 已经授权的不透明身份。语义由 [Cloud 拥有的内部 gRPC 契约](https://github.com/ora-space/specs/blob/main/decisions/cloud/controller-integration/0-cloud-owned-internal-grpc-contract.md)和 [Cloud 拥有契约、Controller 拨出](https://github.com/ora-space/specs/blob/main/decisions/controller/api-boundary/20260922-cloud-owned-contract-and-controller-dial-out.md)两份 ADR 确定；Cloud 自己的视角见其 `docs/runtime/controller-integration` 文档。Controller 如何使用每个服务见 [Controller](controller.zh.md) 和 [Agent 中继](agent-relay.zh.md)。

## 服务

| 服务 | 方法 | 要点 |
|---|---|---|
| `ControllerLeaseService` | `AcquireLease`、`RenewLease`、`ReleaseLease` | 一个全局协调租约；它的 `epoch` 为每次写入设置栅栏 |
| `ExecutionService` | `ClaimWork`、`RecordDispatch`、`TakeOverNodeEvent`、`RecordQueriedResult`、`GetDispatch`、`ListPendingDispatches` | 写入带 `submission_id`：相同内容回放原结果，不同内容以 `ABORTED` + `CONFLICT` 失败。只有 `RecordDispatch` 之后才派发，只有 `TakeOverNodeEvent` 之后才确认；`RecordQueriedResult` 从不授权确认 |
| `ControlSignalService` | `Watch`（服务端流） | `WorkAvailable`、`OperationAvailable`、`Drain`、`NodeAssignment`；至多一次，不持久化，不改变任何归属 |
| `WorkspaceOperationService` | `ClaimOperation`、`PlanEffect`、`RecordEffectResult`、`AdvanceOperation`、`DeferOperation`、`RecordProvisioningFact`、`ListLiveSandboxes` | 写入以 `epoch` 和操作的 `version` 设置栅栏；clone 和 plugin 步骤以操作的 id 通过 `ExecutionService` 派发；`RecordProvisioningFact` 是 Cloud 按 blocked 截止放弃创建后唯一仍接受迟到事实的入口，把一条迟到观测补记到残留账本，不重开任何东西 |
| `NodeReportService` | `RegisterNode`、`ReportNodeStatus`、`ReportNodeIdle`、`EndNode` | Controller 持有会话的沙盒 Node；`RegisterNode` 按（沙盒、incarnation）幂等，第一个结束之前第二个活跃的 incarnation 会冲突 |
| `AgentRunService` | `TakeOverThreadEvents`、`ClaimThreadCommands`、`RecordThreadCommandDelivered`、`GrantRevisionUpload`、`GrantRevisionDownload` | Thread 中继和 Revision 授权 |
| `RuntimeControlService` | `ListBindings`、`AcknowledgeBinding`、`GetExecutionPermit`、`GetEffectPermit`、`ListForceStops`、`ConfirmForceStop` | 带栅栏的运行时控制，见[运行时控制](runtime-control.zh.md) |

失败以 gRPC 状态码作为主要分类，并附带 `ErrorDetail { ErrorCode }`。Rust 一侧在 Cloud RPC 适配器内部一次性把它们映射为协调类别（冲突、缺失、不可用、未知、资格过期），所以协调逻辑从不看到 gRPC。Cloud 对过期的操作快照（`stale_operation`）返回与过期租约相同的 `FAILED_PRECONDITION`；适配器把它与租约判定、普通冲突都区分开：只有租约判定才会丢弃持有的 epoch，也只有操作已关闭而无法接收的 effect 观测会改经 provisioning fact 补记。

## Revision 上传校验和

`GrantRevisionUploadRequest.checksums` 把每个固定的对象 key 映射到它的小写 SHA-256。返回的每个授权都签名 `x-amz-checksum-sha256` 和 `If-None-Match: *`，所以未过期的授权也不能覆盖 Cloud 已经校验过的对象；得到 412 的重试保留第一个对象，并提交原始元数据供校验。上传方在 PUT 中发送返回的每个 header。授权留在内存中，从不进入日志、环境变量、参数或文件，部署用的原始 S3 凭据从不到达 Node。每个执行在第一次 PUT 之前冻结对象字节和元数据，所以重启后用新授权复用它们；内容变化需要新的 Cloud 工作条目和 key。Cloud 校验存在性、大小和存储的 SHA-256，然后原子地记录 Node 证据、回执和 Revision。

恢复运行扩展了同一契约：`AgentSessionSpec.prior_revision` 指明上一个 Revision 及其经过校验的 bundle 对象，`DeliverRevisionSpec.prior_revision` 只有它的最终 commit，`GrantRevisionDownload` 为一个已登记且没有结果的会话签名对那个 bundle 的只在内存中使用的 `GET`。

## 获取与生成契约

`third_party/cloud` 是 Cloud 仓库的 git submodule，它的 gitlink 固定了契约的 commit。工作树是部分 clone（`--filter=blob:none`），sparse checkout 只包含 `proto/`。

- `task proto:init`（Linux 和 macOS）clone 并检出固定的 commit，或把已初始化的 submodule 移到该 commit。它使用 `PATH` 或 `~/.local/bin` 中的 `buf`，没有时从官方 GitHub release 安装 Buf 到 `~/.local/bin`，不需要 sudo（需要网络）。普通的 `git clone` 不会初始化 submodule。Windows 构建使用已提交的生成客户端，既不需要 submodule 也不需要 `buf`。
- `crates/controller-proto`（`ora-controller-proto`）只保存 `src/gen/` 下的生成代码，由 `buf` 用固定版本的远程插件（`neoeinstein-prost`、`neoeinstein-tonic`）生成，不需要本地 `protoc`。服务端 stub 在 `test-server` feature 之后生成：生产代码从不编译它们，Controller 的测试用它们在真实契约上托管一个内存中的 Cloud。
- `task proto:generate` 重新生成。`task proto:check`（属于 `task lint:crates`）检查 submodule 处于固定的 commit 且 `proto/` 下没有本地改动，重新生成，有任何差异就失败。`buf lint` 和 `buf breaking` 属于 Cloud 仓库。

## 升级契约

1. Cloud 合并契约变更（`v1` 只接受增量变更；破坏性变更是同级的 `v2`）。
2. `git -C third_party/cloud fetch origin <commit> && git -C third_party/cloud checkout <commit>`；该 commit 必须在 Cloud 的 main 或受保护分支上。
3. 运行 `task proto:generate`，适配适配器，把 gitlink、生成代码和改动一起提交，并在 PR 中链接 Cloud 的变更。

生成只证明结构一致；与真实 Cloud 服务端的行为一致性由 cluster 仓库的 Compose 环境（Gateway → Cloud → Controller → Sandbox Server → Node）来验证。
