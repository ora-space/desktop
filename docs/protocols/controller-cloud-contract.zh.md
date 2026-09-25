# Controller–Cloud 契约

[English](controller-cloud-contract.md) | 中文

Controller 在云端部署中通过 gRPC 调用 Cloud（Ora Cloud，Go API Server）：领取工作、派发前登记、
接管 Node 事件、保存查询结果、恢复读取，以及一条由 Controller 发起的服务端流接收 Cloud 的信号。
契约的唯一来源是 **Cloud 仓库** 的 `proto/ora/cloud/internal/v1/`（package `ora.cloud.internal.v1`）；
Cloud 是全部服务的服务端并拥有权威持久化，Controller 只拨出，不向 Cloud 暴露任何 gRPC 服务。
本仓库不复制 `.proto`，只持有由锁定 commit 生成的 tonic 代码，生产代码只使用其中的**客户端**。租户留在 Cloud：契约只携带
Cloud 已授权的 opaque 身份，没有 tenant、user 或 membership 字段。

语义由 specs 的 `decisions/cloud/controller-integration/0-cloud-owned-internal-grpc-contract.md`
拥有，本仓库的消费方式由 `decisions/controller/api-boundary/20260922-cloud-owned-contract-and-controller-dial-out.md`
固定；本页只讲本仓库如何取得契约、如何生成、如何升级。运行时接入是
[Controller 运行时](../controller/local-runtime.zh.md) 描述的 `CloudStore` 适配器：租约、`ExecutionService`
与 `Watch` 流均已接入，由流决定何时领取，规则见
`decisions/controller/api-boundary/20260924-controller-consumes-watch-signals.md`；其持久语义遵循
`decisions/controller/persistence/20260922-coordination-store-with-sqlite-and-cloud-adapters.md`。

## 服务与语义要点

| 服务                     | 方法                                                                                                              | 要点                                                                                                                                                                                          |
| ------------------------ | ----------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `ControllerLeaseService` | `AcquireLease`／`RenewLease`／`ReleaseLease`                                                                      | 全局协调租约；`epoch` 是所有写操作的 fencing token                                                                                                                                            |
| `ExecutionService`       | `ClaimWork`、`RecordDispatch`、`TakeOverNodeEvent`、`RecordQueriedResult`、`GetDispatch`、`ListPendingDispatches` | 写操作携带 `submission_id`：同身份同内容返回原结果，不同内容 `ABORTED`+`CONFLICT`；`RecordDispatch` 成功后才可派发，`TakeOverNodeEvent` 成功后才可 Ack，`RecordQueriedResult` 不产生 Ack 依据 |
| `ControlSignalService`   | `Watch`（服务端流）                                                                                               | `WorkAvailable`／`Drain`／`NodeAssignment`；至多一次、不持久化、不改变归属；断流退回周期 `ClaimWork`                                                                                          |

失败以 gRPC 状态码为主分类并附 `ErrorDetail{ErrorCode}`；Rust 侧在 Cloud RPC 适配器内把它们映射
一次为持久协调接口的分类（冲突、缺失、不可用、未知、资格失效），协调逻辑不感知 gRPC。

## 契约的获取：submodule + sparse-checkout

`third_party/cloud` 是指向 Cloud 仓库的 git submodule，gitlink 锁定契约 commit；工作区用 partial
clone（`--filter=blob:none`）与 sparse-checkout 只展开 `proto/`。

- `task proto:init`（Linux／macOS；生成物已提交，Windows 构建不需要 submodule 与 buf）：首次以 `--no-checkout --filter=blob:none --sparse` clone，`sparse-checkout set proto`，
  再 `git submodule update --init` 到锁定 commit；已初始化时只移动到锁定 commit。CI 的 crates job 执行同一任务。
  优先复用 `PATH` 或 `~/.local/bin` 中的 `buf`；缺失时用 `curl` 从
  [官方 GitHub Release](https://buf.build/docs/cli/installation/) 下载 Buf 1.73.0 到 `~/.local/bin`，
  无需 sudo，下载需要网络。协议任务会把此目录加入 `PATH`；如需在终端直接运行 `buf`，请将其加入 shell 的 `PATH`。
- 普通 `git clone` 或 `actions/checkout` 不会初始化它；依赖初始化是显式动作。
- `/specs` 仍是被忽略的独立 checkout，不作为契约依赖。

## 生成与检查

`crates/controller-proto`（`ora-controller-proto`）只放 `src/gen/` 下的生成物，由 `buf` 用固定版本的
远程插件（`neoeinstein-prost`、`neoeinstein-tonic`）从 `third_party/cloud/proto` 生成；需要网络，不需要
本机 `protoc`。服务端模块生成在 `#[cfg(feature = "test-server")]` 之后：生产构建从不编译它们，Controller
的测试启用该 feature，以真实契约托管内存假 Cloud。

- `task proto:generate`：重新生成。
- `task proto:check`：先验证 submodule 位于锁定 commit 且 `proto/` 无本地修改，再重新生成并在有 diff
  时失败；已纳入 `task lint:crates`。
- `buf lint` 与 `buf breaking` 属于 Cloud 仓库，本仓库不重复执行。

## 升级契约

1. Cloud 合入契约变更（其 `v1` 只接受非破坏性修改，破坏性变更以并列 `v2` 表达）。
2. `git -C third_party/cloud fetch origin <commit> && git -C third_party/cloud checkout <commit>`，
   引用的 commit 必须位于 Cloud 主干或受保护分支上、长期可获取。
3. `task proto:generate`，修改适配器，把 gitlink、生成物与代码一起提交；PR 描述链接 Cloud 侧变更以便展开
   子模块 diff 评审。

生成证明的是结构一致。Controller 的测试以生成的服务端桩构建内存假 Cloud 验证适配器行为；与真实 Cloud gRPC
服务端的行为一致经 minicloud 云端形态端到端验证，尚未自动化。
