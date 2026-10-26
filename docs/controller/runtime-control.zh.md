# 运行时控制

[English](runtime-control.md) | 中文

在 Cloud 模式下，谁可以对一个运行时 Workspace 动手由 Cloud 决定；Controller 只交付 Cloud 授权的内容，Node 在它真实的入口处隔离过期的资格。决策是[带栅栏的运行时控制交付](https://github.com/ora-space/specs/blob/main/decisions/cloud/controller-integration/20260927-fenced-runtime-control-delivery.md) ADR；租户角色策略和独占控制属于 Cloud（见 Cloud 的 `docs/runtime/runtime-control` 文档），Controller 和 Node 不复制它们。

## 身份与代次

每个身份回答不同的问题，谁也不能代替谁：

| 身份 | 权威 |
|---|---|
| 用户与成员身份 | Cloud 的认证与授权 |
| Controller 租约 epoch | 当前协调者的写入资格 |
| Workspace 运行时代次与沙盒 id | 一个运行实例及其持久目标 |
| 运行时控制 epoch 与会话 | 一次用户或维护操作的资格 |
| Node 与宿主 incarnation | 一个运行中的程序实例 |
| execution 与 effect id | 对固定输入的持久责任 |

稳定的 execution id 不是新的授权，也不能拿旧许可换个 id 启动第二个任务。改变代次只拒绝新动作，从不证明旧进程已经停止。

## 经过认证的管理通道

Cloud–Controller 和 Controller–Node 两段管理链路都认证双向 TLS 的服务身份和目标绑定；消息中声明的 id 只用于一致性检查。Controller 拒绝没有 `management_tls`、Cloud endpoint 不是 HTTPS、effects 不是 HTTPS 或路由器不是 WSS 的 cloud 配置，见 [Controller](controller.zh.md#cloud-形态)。Controller 只拨出，不在 SQLite 中保存任何 Cloud 业务状态。

在 Node 镜像中，root 拥有的管理进程（Node 和进程宿主）把配置、日志和私钥放在只有 root 可访问的 `/run/ora` 下，Git 工作负载和 Agent 插件以 UID/GID 1000 运行，无法读取管理材料或写入控制日志，见[容器镜像](../operations/container-images.zh.md)。静态 Git 配置只能包含非机密的 CA 信任，绝不能有个人凭据助手或 SSH 后备。

## 派发、关闭与回报

1. Cloud 在一个事务里检查权限、控制、运行实例和冲突的活动，并保存固定的执行输入。
2. 每次新派发之前，Controller 获取一个新许可（`GetExecutionPermit`，Substrate effect 用 `GetEffectPermit`）。Cloud 不可用或许可无效时，不启动任何新动作。
3. Node 先持久化受理再执行。以相同输入重传同一身份只返回已有的状态。不带栅栏的裸命令只在从未启用过运行时控制的 Node home 上、通过私有本地 IPC 才被接受；一旦某个 home 启用过，这条后路就永久关闭。

撤回资格会关闭新的准入：Controller 把 input-closed 绑定送达 Node（`ListBindings`、`AcknowledgeBinding`），未受理的旧动作被拒绝，已受理的工作在原来的范围内收尾。Node 回报关闭以及未完成的 execution id，Cloud 对账之后才移交控制权。关闭在一个 epoch 内是不可逆的。丢失的响应保持未知，从不根据缓存放行任何东西；用户失去权限也不会丢弃一个已在运行的执行的结束事实。

Node 持久化它的运行时目标、控制 epoch 与版本、关闭状态和责任。更低的 epoch、同一 epoch 下不同的绑定、错误的 incarnation 或错误的目标都会被拒绝，在受理队列和第一次真实变更处都会检查，重启也从不在内存中重新打开旧入口。

## 已截止交付的未受理证明

Cloud 的业务结算不等于 Node 终态。Controller 只在 Hello 明确启用并由 Node 声明
`execution_non_acceptance` 后，请求 `ListExecutionFences` 中的原派发身份。旧 Controller 不会收到
陌生 capability；旧 Node 缺少能力时保留未知责任，不回退到裸命令或 Unknown 推断。

Node 在同一 SQLite writer 事务中检查完整身份 ledger，并为未受理的 execution 写入永久输入围栏。
该围栏不受新 control epoch、迟到 permit 或重启影响，也不关闭另一交付族。已受理/已完成不产生证明；
新 incarnation 的空库不能证明旧实例未受理，旧实例已持久证明则可原样重放。

`RecordExecutionNonAcceptance` 独立接管证明，不产生普通 event ACK。Controller 只有在提交成功后才
清除对应 unresolved；丢失响应由 RPC 返回的已提交证明身份恢复。quiesce 继续关闭全局派发并检查
clone、插件、会话、Revision、产物上传及剩余未知责任，Cloud 的行政结算本身不能跳过其中任何一项。

## 强制停止

紧急终止使用独立的 effect 许可，最长 10 秒，并且从不超过 Controller 的租约。Controller 的强制停止 worker（`ListForceStops`、`ConfirmForceStop`）在普通生命周期步骤等待 Node 时仍持续交付，从不编造执行完成。执行一侧使用持久的运行时栅栏，防止迟到的 ensure 重新创建目标；只有同时证明 `terminated` 和 `lateEnsureFenced` 时，Cloud 才接受终止。不可达、心跳超时或新的代次都不允许对同一目录的并行写入。

Controller 不按请求者角色筛选强停计划：同一 RPC 可交付不同 Cloud 入口产生的计划，每项保留原 effect 和 sandbox ID，包括从未执行 ensure 的目标。单个目标拒绝、执行失败或 Substrate 不可达时继续其他独立目标；获取许可或确认时遇到 Cloud 不可用、或本地租约丢失，则停止新派发。每个新 PUT 在 journal GET 之后检查许可 epoch、有效期和当前本地租约；Node 控制关闭耗时之后也重新检查，不能延长许可。本地派发要求许可剩余有效期超过 2 秒，与 Sandbox Server 一致。生命周期 effect 在同一 epoch 内遇到许可过期时进入 RetryWait，下次尝试重新获取许可并沿原 effect 继续；epoch 不符时结束本轮，不写入 defer 状态。

新 terminate PUT 前，worker 尽力移除已有 Fleet target 并发送现有 Shutdown control close；没有连接或 close 没有确认不会阻止后续终止。该消息不是 EndSession，也不是终止证据；EndSession 仍只能来自已有 Cloud 命令队列及其授权。GET404、Node 不可达、过期许可、缺失或为 false 的双确认字段均不能产生终止确认。ensure 收到精确的 `sandbox_terminated`（503 准入错误，或 failed journal 顶层 error）时进入已有 Blocked 路径，恢复 GET 也不能重新 PUT。普通 503、Docker 失败和 stale permission 不会被改称墓碑；已有成功 journal 仍保持原历史结果。

`ora-controller` 的 `src/cloud/substrate/put_admission.rs`、`tests/workspaces/force_stop.rs`、`tests/workspaces/permit_expiry.rs` 和 `src/cloud/operations/eligibility.rs` 中的测试通过真实 HTTP/gRPC/WebSocket 协议与本地假服务覆盖许可、控制关闭、终止证据和墓碑拒绝边界；它们不证明生产授权、真实容器销毁或 Cloud–Controller–Substrate 的联合故障恢复。Cloud 未提供的目标不由 Controller 自动补造。

## 兼容性

对端显式协商 `runtime_control`，受控的执行从不悄悄退回到不带绑定的请求。已退役的监听和旧的 JSON 接口保持关闭，历史责任仍可查询。升级保留历史的 owner、执行和审计引用；回滚从不通过删除新的持久状态来实现。Node 证书有效期短且没有无缝续期，轮换需要受控的重启。旧的由工作负载拥有的管理 home 不会被自动修复：未完成的未绑定历史会阻止新的绑定，部署必须先停止并对账旧进程，再做受控迁移。
